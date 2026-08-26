//! Reading the unix seconds that marks, beats and the activity ledger are
//! written in back into a local instant.
//!
//! Those files store bare epochs so a shell hook can append one with `date
//! +%s`; every reader has to turn them back into something with a timezone.

use chrono::{DateTime, Local};

/// One epoch as a local instant, or `None` when it is not a valid timestamp.
///
/// The `Option` is the honest shape: a mark file is plain text that anything
/// could have written. Callers that owe the operator an explanation wrap this
/// with their own context — see `agent::instant`.
pub fn instant(epoch: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(epoch, 0).map(|instant| instant.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn an_epoch_reads_back_as_the_same_instant() {
        let when = Local.with_ymd_and_hms(2026, 3, 25, 9, 30, 0).unwrap();
        assert_eq!(instant(when.timestamp()), Some(when));
    }

    #[test]
    fn the_epoch_itself_is_valid() {
        assert!(instant(0).is_some());
    }

    #[test]
    fn an_out_of_range_timestamp_is_none_rather_than_a_panic() {
        assert_eq!(instant(i64::MAX), None);
    }
}
