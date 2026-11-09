use std::time::Duration;
use uuid::Uuid;

/// A trusted adapter binds to the value returned by `MicrophoneGate::enable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuteBinding {
    pub capture_generation: Uuid,
    pub observer_generation: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteSignal {
    Muted,
    Unmuted,
    Unknown,
}

/// All times use the same capture-relative monotonic clock. Adapters must also
/// invalidate on sleep, permission loss, target loss and session changes.
#[derive(Clone, Copy, Debug)]
pub struct SampleWindow {
    pub start: Duration,
    pub end: Duration,
    pub received: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicDecision {
    Capture,
    ManualPause,
    Unknown,
    Muted,
    Stale,
    Revoked,
    TimingUnavailable,
}

/// A bounded policy, not a mute detector. Callers must serialize policy updates
/// with sample writes; the writer's `&mut self` boundary enforces this locally.
pub struct MicrophoneGate {
    binding: Option<MuteBinding>,
    paused: bool,
    revoked: bool,
    state: MuteSignal,
    sequence: Option<u64>,
    observed: Option<Duration>,
    open_since: Option<Duration>,
    deadline: Option<Duration>,
    floor: Option<Duration>,
    revision: u64,
}

/// Short lease: a stopped detector cannot silently leave the mic open.
pub const MUTE_SIGNAL_TTL: Duration = Duration::from_secs(2);

impl Default for MicrophoneGate {
    fn default() -> Self {
        Self::ordinary()
    }
}

impl MicrophoneGate {
    /// Existing recording behavior. No automatic protection is claimed.
    pub fn ordinary() -> Self {
        Self {
            binding: None,
            paused: false,
            revoked: false,
            state: MuteSignal::Unknown,
            sequence: None,
            observed: None,
            open_since: None,
            deadline: None,
            floor: None,
            revision: 0,
        }
    }

    /// Every opt-in creates a fresh observer generation, including re-enabling
    /// in the same capture. Previously queued observations cannot be replayed.
    pub fn enable(&mut self, capture_generation: Uuid, now: Duration) -> MuteBinding {
        let binding = MuteBinding {
            capture_generation,
            observer_generation: Uuid::new_v4(),
        };
        let paused = self.paused;
        let revision = self.revision.wrapping_add(1);
        *self = Self::ordinary();
        self.paused = paused;
        self.revision = revision;
        self.binding = Some(binding);
        self.floor = Some(now);
        binding
    }

    /// Explicit opt-out does not override a manual pause or release queued audio
    /// from an earlier blocked interval. Such frames still need valid timing.
    pub fn disable(&mut self, now: Duration) {
        self.binding = None;
        self.floor = Some(now);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn pause(&mut self, paused: bool, now: Duration) {
        if self.paused != paused {
            self.paused = paused;
            self.floor = Some(now);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Permission/target/session loss is terminal until a new explicit enable.
    pub fn revoke(&mut self) {
        self.revoked = true;
        self.open_since = None;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Accept only newer observations for this binding. Invalid timestamps
    /// consume their sequence but close the gate; stale packets cannot reopen it.
    pub fn observe(
        &mut self,
        binding: MuteBinding,
        sequence: u64,
        observed: Duration,
        received: Duration,
        signal: MuteSignal,
    ) -> bool {
        if self.binding != Some(binding)
            || self.revoked
            || self.sequence.is_some_and(|last| sequence <= last)
        {
            return false;
        }
        self.sequence = Some(sequence);
        let fresh = received
            .checked_sub(observed)
            .is_some_and(|age| age < MUTE_SIGNAL_TTL)
            && self.observed.is_none_or(|last| observed >= last)
            && self.floor.is_none_or(|floor| observed >= floor);
        let signal = if fresh { signal } else { MuteSignal::Unknown };
        let continuous = signal == MuteSignal::Unmuted
            && self.state == signal
            && self.deadline.is_some_and(|end| received < end);
        if !continuous {
            self.open_since = (signal == MuteSignal::Unmuted).then_some(received);
            self.revision = self.revision.wrapping_add(1);
        }
        if fresh {
            self.observed = Some(observed);
        }
        self.deadline = if fresh {
            observed.checked_add(MUTE_SIGNAL_TTL)
        } else {
            None
        };
        self.state = signal;
        true
    }

    pub fn decision(&self, frame: Option<SampleWindow>) -> MicDecision {
        if self.paused {
            return MicDecision::ManualPause;
        }
        if self.binding.is_some() && self.revoked {
            return MicDecision::Revoked;
        }
        if self.binding.is_some() {
            match self.state {
                MuteSignal::Muted => return MicDecision::Muted,
                MuteSignal::Unknown => return MicDecision::Unknown,
                MuteSignal::Unmuted => {}
            }
        }
        if self.binding.is_none() && self.floor.is_none() {
            return MicDecision::Capture;
        }
        let Some(frame) = frame else {
            return MicDecision::TimingUnavailable;
        };
        if frame.start > frame.end
            || frame.end > frame.received
            || self.floor.is_some_and(|floor| frame.start < floor)
        {
            return MicDecision::TimingUnavailable;
        }
        if self.binding.is_some() {
            if self.deadline.is_none_or(|end| frame.received >= end) {
                return MicDecision::Stale;
            }
            if self.open_since.is_none_or(|start| frame.start < start) {
                return MicDecision::TimingUnavailable;
            }
        }
        MicDecision::Capture
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
}
