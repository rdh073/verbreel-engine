//! Error-callback surfacing: the pure decision the wasm32 decode loop
//! runs before draining frames.
//!
//! The `WebCodecs` `error` callback writes a browser `DOMException`
//! message into a shared [`ErrorSlot`]; a later `WebCodecsSession::drain`
//! must surface it as [`DecodeError::DecoderInternal`] *before* touching
//! the frame queue, so a fatal-decode session reports the failure
//! instead of silently returning a partial frame set.
//!
//! That decision is pure (`Rc<RefCell<Option<String>>>` carries no
//! browser dependency), so it lives here and is exercised by native
//! unit tests — mirroring `crate::frame::timestamp_to_micros`. The
//! wasm32-only `webcodecs` module owns only the `web_sys` glue that
//! cannot be faulted without a browser.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::DecodeError;

/// Shared last-error slot. The `WebCodecs` `error` callback writes the
/// browser's `DOMException` message here so a later drain can surface
/// it as [`DecodeError::DecoderInternal`].
pub(crate) type ErrorSlot = Rc<RefCell<Option<String>>>;

/// Take any pending browser error out of `slot`, mapping it to
/// [`DecodeError::DecoderInternal`].
///
/// Returns `Some(err)` when the `error` callback recorded a failure —
/// the caller must short-circuit *before* draining frames so the queue
/// is left intact for `Drop` to release. Returns `None` when no error
/// is pending, in which case the caller proceeds to drain.
///
/// The slot is emptied on take so a recovered session does not re-report
/// a stale error on the next drain.
//
// Called from the wasm32-only `webcodecs` decode loop and from the
// native unit tests below; unused on a native non-test lib build.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub(crate) fn take_pending_error(slot: &ErrorSlot) -> Option<DecodeError> {
    slot.borrow_mut()
        .take()
        .map(|detail| DecodeError::DecoderInternal { detail })
}

#[cfg(test)]
mod tests {
    use super::{ErrorSlot, take_pending_error};
    use crate::error::DecodeError;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn empty_slot_yields_none_so_drain_proceeds() {
        let slot: ErrorSlot = Rc::new(RefCell::new(None));
        assert!(take_pending_error(&slot).is_none());
    }

    #[test]
    fn pending_error_surfaces_as_decoder_internal_with_detail() {
        let slot: ErrorSlot = Rc::new(RefCell::new(Some("EncodingError: fatal".to_string())));
        match take_pending_error(&slot) {
            Some(DecodeError::DecoderInternal { detail }) => {
                assert_eq!(detail, "EncodingError: fatal");
            }
            other => panic!("expected DecoderInternal, got {other:?}"),
        }
    }

    #[test]
    fn take_empties_the_slot_so_a_recovered_session_does_not_re_report() {
        // Models the error-callback -> last_error -> drain surfacing
        // path: the slot is seeded (as the `error` callback would), the
        // first drain surfaces it, and a subsequent drain on a recovered
        // session sees a clean slot. A regression that returned `Ok`
        // here (dropping the early-return) would leave this assertion's
        // first arm unsatisfied.
        let slot: ErrorSlot = Rc::new(RefCell::new(Some("DOMException".to_string())));
        assert!(matches!(
            take_pending_error(&slot),
            Some(DecodeError::DecoderInternal { .. })
        ));
        assert!(
            take_pending_error(&slot).is_none(),
            "slot must be empty after the error is surfaced once"
        );
    }
}
