package cron_test

import (
	cron "github.com/robfig/cron/v3"
	"testing"
)

func TestPauseSmoke(t *testing.T) {
	c := cron.New()
	id, err := c.AddFunc("@every 1h", func() {})
	if err != nil {
		t.Fatal(err)
	}
	if !c.Pause(id) || !c.Entry(id).Paused || !c.Entry(id).Next.IsZero() {
		t.Fatal("Pause must retain the entry in a dormant state")
	}
	if !c.Resume(id) || c.Entry(id).Paused || !c.Entry(id).Next.IsZero() {
		t.Fatal("Resume while stopped must reactivate without scheduling")
	}
}

func TestPauseUnknownSmoke(t *testing.T) {
	c := cron.New()
	if c.Pause(0) || c.Resume(0) || c.Pause(999) || c.Resume(999) {
		t.Fatal("unknown IDs cannot change state")
	}
}
