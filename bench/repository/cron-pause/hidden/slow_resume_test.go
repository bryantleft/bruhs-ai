package cron_test

import (
	cron "github.com/robfig/cron/v3"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

type reviewSchedule func(time.Time) time.Time

func (s reviewSchedule) Next(now time.Time) time.Time { return s(now) }

func TestFrozenSlowResumePreservesOtherDeadline(t *testing.T) {
	c := cron.New()
	entered, release := make(chan struct{}), make(chan struct{})
	var once sync.Once
	unblock := func() { once.Do(func() { close(release) }) }
	defer func() { unblock(); <-c.Stop().Done() }()
	var calls int32
	target := c.Schedule(reviewSchedule(func(now time.Time) time.Time {
		if atomic.AddInt32(&calls, 1) == 2 {
			close(entered)
			<-release
		}
		return now.Add(time.Hour)
	}), cron.FuncJob(func() {}))
	deadline := make(chan time.Time, 1)
	fired := make(chan time.Time, 1)
	var otherCalls int32
	c.Schedule(reviewSchedule(func(now time.Time) time.Time {
		if atomic.AddInt32(&otherCalls, 1) == 1 {
			next := now.Add(2 * time.Second)
			deadline <- next
			return next
		}
		return time.Time{}
	}), cron.FuncJob(func() { fired <- time.Now() }))
	c.Start()
	due := <-deadline
	c.Pause(target)
	resumed := make(chan bool, 1)
	go func() { resumed <- c.Resume(target) }()
	select {
	case <-entered:
	case <-time.After(3 * time.Second):
		t.Fatal("Resume did not enter its schedule")
	}
	// Release only after the unrelated deadline is already overdue. The 1s
	// observation window distinguishes prompt dispatch from rearming a stale
	// roughly 2s timer. It is a mechanism probe, not a product latency SLA.
	time.Sleep(time.Until(due) + 500*time.Millisecond)
	released := time.Now()
	unblock()
	select {
	case ok := <-resumed:
		if !ok {
			t.Fatal("Resume failed")
		}
	case <-time.After(3 * time.Second):
		t.Fatal("Resume did not finish")
	}
	select {
	case ran := <-fired:
		t.Logf("unrelated overdue job ran %s after schedule release", ran.Sub(released))
	case <-time.After(time.Second):
		t.Fatal("unrelated overdue job was given an additional timer delay after Resume finished")
	}
}
