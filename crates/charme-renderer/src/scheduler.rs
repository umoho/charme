/// Decision returned when the viewport scheduler is ready to start a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameRequest {
    /// Whether one Bevy update must run before scheduling the readback.
    pub(crate) prepare_selection: bool,
}

/// Small state machine that coalesces viewport invalidations and owns readback state.
///
/// Background preview queues remain owned by the renderer backend, but may only
/// start while this scheduler reports that the viewport is idle.
#[derive(Debug, Default)]
pub(crate) struct RenderScheduler {
    viewport_dirty: bool,
    frame_in_flight: bool,
    selection_dirty: bool,
}

impl RenderScheduler {
    pub(crate) fn invalidate(&mut self) {
        self.viewport_dirty = true;
    }

    pub(crate) fn invalidate_selection(&mut self) {
        self.viewport_dirty = true;
        self.selection_dirty = true;
    }

    pub(crate) fn complete_frame(&mut self) {
        self.frame_in_flight = false;
    }

    pub(crate) const fn has_pending_viewport_work(&self) -> bool {
        self.viewport_dirty || self.frame_in_flight
    }

    pub(crate) const fn viewport_is_idle(&self) -> bool {
        !self.viewport_dirty && !self.frame_in_flight
    }

    pub(crate) const fn frame_in_flight(&self) -> bool {
        self.frame_in_flight
    }

    /// Records that Bevy has advanced far enough to materialize pending gizmos.
    pub(crate) fn app_updated(&mut self) {
        self.selection_dirty = false;
    }

    /// Starts a coalesced viewport readback when possible.
    pub(crate) fn take_frame_request(&mut self, suspended: bool) -> Option<FrameRequest> {
        if !self.viewport_dirty {
            return None;
        }
        if suspended {
            self.viewport_dirty = false;
            self.selection_dirty = false;
            return None;
        }
        if self.frame_in_flight {
            return None;
        }

        let request = FrameRequest {
            prepare_selection: self.selection_dirty,
        };
        self.viewport_dirty = false;
        self.selection_dirty = false;
        self.frame_in_flight = true;
        Some(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_invalidations_into_one_frame() {
        let mut scheduler = RenderScheduler::default();
        scheduler.invalidate();
        scheduler.invalidate();

        assert_eq!(
            scheduler.take_frame_request(false),
            Some(FrameRequest {
                prepare_selection: false
            })
        );
        assert_eq!(scheduler.take_frame_request(false), None);

        scheduler.complete_frame();
        assert!(scheduler.viewport_is_idle());
    }

    #[test]
    fn keeps_an_invalidation_received_during_readback() {
        let mut scheduler = RenderScheduler::default();
        scheduler.invalidate();
        scheduler.take_frame_request(false).unwrap();
        scheduler.invalidate();

        assert_eq!(scheduler.take_frame_request(false), None);
        scheduler.complete_frame();
        assert!(scheduler.take_frame_request(false).is_some());
    }

    #[test]
    fn selection_requests_prepare_the_first_frame() {
        let mut scheduler = RenderScheduler::default();
        scheduler.invalidate_selection();

        assert!(
            scheduler
                .take_frame_request(false)
                .unwrap()
                .prepare_selection
        );
    }

    #[test]
    fn suspended_outputs_discard_pending_frames() {
        let mut scheduler = RenderScheduler::default();
        scheduler.invalidate_selection();

        assert_eq!(scheduler.take_frame_request(true), None);
        assert!(scheduler.viewport_is_idle());
    }
}
