package cron_test

import (
	cron "github.com/robfig/cron/v3"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// Three seconds covers three upstream one-second schedule ticks; it bounds
// fixture liveness, not a product responsiveness or throughput claim.
const frozenWait = 3 * time.Second

type frozenSchedule struct {
	mu     sync.Mutex
	times  []time.Time
	delay  time.Duration
	first  time.Duration
	zero   bool
	called chan struct{}
}

func (s *frozenSchedule) Next(now time.Time) time.Time {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.times = append(s.times, now)
	if s.called != nil {
		select {
		case s.called <- struct{}{}:
		default:
		}
	}
	if s.zero {
		return time.Time{}
	}
	delay := s.delay
	if len(s.times) == 1 && s.first != 0 {
		delay = s.first
	}
	return now.Add(delay)
}

func (s *frozenSchedule) observations() []time.Time {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]time.Time(nil), s.times...)
}

func frozenAwait(t *testing.T, ch <-chan struct{}) {
	t.Helper()
	select {
	case <-ch:
	case <-time.After(frozenWait):
		t.Fatal("scheduler did not make progress")
	}
}

func frozenStop(t *testing.T, c *cron.Cron) {
	t.Helper()
	frozenAwait(t, c.Stop().Done())
}

func TestFrozenPauseTransitions(t *testing.T) {
	c := cron.New()
	id := c.Schedule(&frozenSchedule{delay: time.Hour}, cron.FuncJob(func() {}))
	if !c.Pause(id) || c.Pause(id) {
		t.Fatal("Pause must report only a state transition")
	}
	if !c.Resume(id) || c.Resume(id) {
		t.Fatal("Resume must report only a state transition")
	}
	c.Remove(id)
	if c.Pause(id) || c.Resume(id) || c.Entry(id).Valid() {
		t.Fatal("removed entries cannot be resurrected")
	}
}

func TestFrozenPausedIdentityAndSnapshot(t *testing.T) {
	var wraps int32
	c := cron.New(cron.WithChain(func(j cron.Job) cron.Job { atomic.AddInt32(&wraps, 1); return j }))
	s := &frozenSchedule{delay: time.Hour}
	job := &frozenJob{}
	id := c.Schedule(s, job)
	before := c.Entry(id)
	c.Pause(id)
	after := c.Entry(id)
	if !after.Valid() || !after.Paused || !after.Next.IsZero() || after.ID != before.ID || after.Schedule != s || after.Job != job || after.WrappedJob != before.WrappedJob || after.Prev != before.Prev {
		t.Fatal("pausing changed identity, history or wrapper")
	}
	after.Paused = false
	entries := c.Entries()
	entries[0].Paused = false
	if !c.Entry(id).Paused {
		t.Fatal("snapshot mutation changed scheduler state")
	}
	c.Resume(id)
	if atomic.LoadInt32(&wraps) != 1 || c.Entry(id).WrappedJob != before.WrappedJob {
		t.Fatal("resume rewrapped the job")
	}
}

type frozenJob struct{}

func (*frozenJob) Run() {}

func TestFrozenStartSkipsPausedSchedules(t *testing.T) {
	c := cron.New()
	s := &frozenSchedule{delay: time.Hour}
	id := c.Schedule(s, cron.FuncJob(func() { t.Error("paused job dispatched") }))
	c.Pause(id)
	c.Start()
	defer frozenStop(t, c)
	if !c.Entry(id).Paused || !c.Entry(id).Next.IsZero() || len(s.observations()) != 0 {
		t.Fatal("startup activated a paused schedule")
	}
	if c.Pause(id) {
		t.Fatal("running duplicate pause changed state")
	}
}

func TestFrozenRunSkipsPausedSchedules(t *testing.T) {
	c := cron.New()
	s := &frozenSchedule{delay: time.Hour}
	id := c.Schedule(s, cron.FuncJob(func() { t.Error("paused job dispatched") }))
	c.Pause(id)
	started := make(chan struct{}, 1)
	c.Schedule(&frozenSchedule{delay: time.Hour, called: started}, cron.FuncJob(func() {}))
	go c.Run()
	frozenAwait(t, started)
	defer frozenStop(t, c)
	if !c.Entry(id).Paused || !c.Entry(id).Next.IsZero() || len(s.observations()) != 0 {
		t.Fatal("Run activated a paused schedule")
	}
}

func TestFrozenPauseDoesNotDispatch(t *testing.T) {
	c := cron.New()
	var runs int32
	id := c.Schedule(&frozenSchedule{delay: 20 * time.Millisecond}, cron.FuncJob(func() { atomic.AddInt32(&runs, 1) }))
	c.Pause(id)
	pulse := make(chan struct{}, 8)
	c.Schedule(&frozenSchedule{delay: 20 * time.Millisecond}, cron.FuncJob(func() {
		select {
		case pulse <- struct{}{}:
		default:
		}
	}))
	c.Start()
	frozenAwait(t, pulse)
	frozenAwait(t, pulse)
	frozenStop(t, c)
	if atomic.LoadInt32(&runs) != 0 {
		t.Fatal("paused entry ran while another entry progressed")
	}
}

func TestFrozenResumeUsesFreshClockAndLocation(t *testing.T) {
	location := time.FixedZone("fixture", 7*60*60)
	c := cron.New(cron.WithLocation(location))
	s := &frozenSchedule{delay: time.Hour}
	id := c.Schedule(s, cron.FuncJob(func() {}))
	c.Start()
	defer frozenStop(t, c)
	old := c.Entry(id)
	if old.Next.IsZero() {
		t.Fatal("active entry was not scheduled")
	}
	if !c.Pause(id) {
		t.Fatal("running Pause failed")
	}
	before := time.Now()
	if !c.Resume(id) {
		t.Fatal("running Resume failed")
	}
	after := time.Now()
	seen := s.observations()
	entry := c.Entry(id)
	if len(seen) != 2 || seen[1].Before(before) || seen[1].After(after) || seen[1].Location() != location || entry.Next != seen[1].Add(time.Hour) || entry.Prev != old.Prev || entry.Paused {
		t.Fatal("resume must schedule once from fresh current time in the configured location")
	}
	if c.Resume(id) || len(s.observations()) != 2 {
		t.Fatal("duplicate resume rescheduled an active entry")
	}
}

func TestFrozenHistorySurvivesPauseAndResume(t *testing.T) {
	c := cron.New()
	dispatched := make(chan struct{}, 1)
	id := c.Schedule(&frozenSchedule{first: 10 * time.Millisecond, delay: time.Hour}, cron.FuncJob(func() { dispatched <- struct{}{} }))
	c.Start()
	defer frozenStop(t, c)
	frozenAwait(t, dispatched)
	before := c.Entry(id)
	if before.Prev.IsZero() {
		t.Fatal("executed job has no history")
	}
	c.Pause(id)
	if e := c.Entry(id); !e.Paused || !e.Next.IsZero() || e.Prev != before.Prev {
		t.Fatal("Pause lost execution history")
	}
	c.Resume(id)
	if e := c.Entry(id); e.Paused || e.Next.IsZero() || e.Prev != before.Prev {
		t.Fatal("Resume lost execution history")
	}
}

func TestFrozenStoppedResumeDefersScheduling(t *testing.T) {
	c := cron.New()
	s := &frozenSchedule{delay: time.Hour}
	id := c.Schedule(s, cron.FuncJob(func() {}))
	c.Start()
	_ = c.Entry(id)
	frozenStop(t, c)
	before := len(s.observations())
	c.Pause(id)
	if !c.Resume(id) || !c.Entry(id).Next.IsZero() || len(s.observations()) != before {
		t.Fatal("stopped Resume executed a schedule")
	}
	c.Start()
	defer frozenStop(t, c)
	if c.Entry(id).Next.IsZero() || len(s.observations()) != before+1 {
		t.Fatal("restart did not schedule the reactivated entry once")
	}
}

func TestFrozenPauseSurvivesRestart(t *testing.T) {
	c := cron.New()
	s := &frozenSchedule{delay: time.Hour}
	id := c.Schedule(s, cron.FuncJob(func() {}))
	c.Start()
	_ = c.Entry(id)
	c.Pause(id)
	frozenStop(t, c)
	before := len(s.observations())
	c.Start()
	defer frozenStop(t, c)
	if e := c.Entry(id); !e.Paused || !e.Next.IsZero() || len(s.observations()) != before {
		t.Fatal("restart activated a paused entry")
	}
}

func TestFrozenUnsatisfiableResume(t *testing.T) {
	c := cron.New()
	s := &frozenSchedule{zero: true}
	id := c.Schedule(s, cron.FuncJob(func() { t.Error("unsatisfiable entry ran") }))
	c.Pause(id)
	c.Start()
	defer frozenStop(t, c)
	if !c.Resume(id) {
		t.Fatal("cannot resume a paused unsatisfiable entry")
	}
	if e := c.Entry(id); e.Paused || !e.Next.IsZero() || len(s.observations()) != 1 {
		t.Fatal("unsatisfiable schedule must remain dormant and active")
	}
}

func TestFrozenOtherEntryUnchanged(t *testing.T) {
	c := cron.New()
	a := c.Schedule(&frozenSchedule{delay: time.Hour}, cron.FuncJob(func() {}))
	s := &frozenSchedule{delay: 2 * time.Hour}
	b := c.Schedule(s, cron.FuncJob(func() {}))
	c.Start()
	defer frozenStop(t, c)
	before := c.Entry(b)
	c.Pause(a)
	c.Resume(a)
	after := c.Entry(b)
	if after.Paused || after.Next != before.Next || after.Prev != before.Prev || len(s.observations()) != 1 {
		t.Fatal("another entry was rescheduled by pause/resume")
	}
}

func TestFrozenInflightJobAndStop(t *testing.T) {
	c := cron.New()
	started, release, finished := make(chan struct{}, 1), make(chan struct{}), make(chan struct{}, 1)
	id := c.Schedule(&frozenSchedule{first: 10 * time.Millisecond, delay: time.Hour}, cron.FuncJob(func() { started <- struct{}{}; <-release; finished <- struct{}{} }))
	c.Start()
	frozenAwait(t, started)
	if !c.Pause(id) {
		close(release)
		frozenStop(t, c)
		t.Fatal("cannot pause an in-flight entry")
	}
	ctx := c.Stop()
	select {
	case <-ctx.Done():
		close(release)
		t.Fatal("Stop stopped waiting for a paused in-flight job")
	default:
	}
	close(release)
	frozenAwait(t, finished)
	frozenAwait(t, ctx.Done())
}

func TestFrozenJobCanControlEntries(t *testing.T) {
	c := cron.New()
	ran := make(chan struct{}, 1)
	target := c.Schedule(&frozenSchedule{first: 10 * time.Millisecond, delay: time.Hour}, cron.FuncJob(func() { ran <- struct{}{} }))
	c.Pause(target)
	result := make(chan [2]bool, 1)
	var self cron.EntryID
	self = c.Schedule(&frozenSchedule{first: 10 * time.Millisecond, delay: time.Hour}, cron.FuncJob(func() { result <- [2]bool{c.Pause(self), c.Resume(target)} }))
	c.Start()
	defer frozenStop(t, c)
	select {
	case ok := <-result:
		if !ok[0] || !ok[1] {
			t.Fatal("job control failed")
		}
	case <-time.After(frozenWait):
		t.Fatal("job control deadlocked")
	}
	frozenAwait(t, ran)
	if !c.Entry(self).Paused || c.Entry(target).Paused {
		t.Fatal("job control states were not retained")
	}
}

func TestFrozenRemoveAndAddWhileAllPaused(t *testing.T) {
	c := cron.New()
	id := c.Schedule(&frozenSchedule{delay: time.Hour}, cron.FuncJob(func() {}))
	c.Pause(id)
	c.Start()
	defer frozenStop(t, c)
	_ = c.Entry(id)
	c.Remove(id)
	if c.Resume(id) || c.Pause(id) || c.Entry(id).Valid() {
		t.Fatal("removed paused entry survived")
	}
	ran := make(chan struct{}, 1)
	newID := c.Schedule(&frozenSchedule{first: 10 * time.Millisecond, delay: time.Hour}, cron.FuncJob(func() { ran <- struct{}{} }))
	if newID <= id || c.Entry(newID).Paused {
		t.Fatal("new entry inherited paused identity")
	}
	frozenAwait(t, ran)
}

func TestFrozenConcurrentLifecycleAndSnapshots(t *testing.T) {
	c := cron.New()
	ids := make([]cron.EntryID, 8)
	for i := range ids {
		ids[i] = c.Schedule(&frozenSchedule{delay: time.Hour}, cron.FuncJob(func() {}))
	}
	c.Start()
	var workers sync.WaitGroup
	for _, id := range ids {
		workers.Add(1)
		go func(id cron.EntryID) {
			defer workers.Done()
			for i := 0; i < 40; i++ {
				c.Pause(id)
				_ = c.Entry(id)
				_ = c.Entries()
				c.Resume(id)
			}
		}(id)
	}
	workers.Add(1)
	go func() {
		defer workers.Done()
		for i := 0; i < 20; i++ {
			c.Stop()
			c.Start()
			id := c.Schedule(&frozenSchedule{delay: time.Hour}, cron.FuncJob(func() {}))
			c.Pause(id)
			c.Remove(id)
		}
	}()
	done := make(chan struct{})
	go func() { workers.Wait(); close(done) }()
	frozenAwait(t, done)
	frozenStop(t, c)
	for _, id := range ids {
		if !c.Pause(id) || !c.Entry(id).Paused {
			t.Fatal("settled control state is inconsistent")
		}
	}
}
