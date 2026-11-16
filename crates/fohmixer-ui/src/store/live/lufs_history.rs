//! The LUFS history's reads (#219): `GET /api/lufs/history` and
//! `GET /api/lufs/days` with the page's token. The URLs and the answers are
//! the tested `behave::lufs_history`; this is the fetch alone.

use fohmixer_proto::lufs::{HistoryDay, HistorySeries};

use super::LiveStore;
use crate::behave::lufs_history::{days_answer, days_url, history_url, series_answer};
use crate::{dom, net};

impl LiveStore {
    /// A point's history over `from..to` (UTC ms); none when it could not
    /// be read (logged with why).
    pub async fn lufs_history(
        self,
        instance: &str,
        target: &str,
        from: i64,
        to: i64,
    ) -> Option<HistorySeries> {
        let url = history_url(instance, target, from, to);
        let answer = net::fetch_text("GET", &url, Some(&self.token()), None).await;
        let read = answer.and_then(|(status, body)| series_answer(status, &body));
        match read {
            Ok(series) => {
                dom::log(&format!(
                    "lufs history {instance} {target} {from}..{to}: {} values of {} ms",
                    series.values.len(),
                    series.step_ms
                ));
                Some(series)
            }
            Err(why) => {
                dom::log(&format!(
                    "lufs history {instance} {target} {from}..{to} not read: {why}"
                ));
                None
            }
        }
    }

    /// The days a point's history holds, newest first; none when they could
    /// not be read (logged with why).
    pub async fn lufs_days(self, instance: &str, target: &str) -> Option<Vec<HistoryDay>> {
        let url = days_url(instance, target);
        let answer = net::fetch_text("GET", &url, Some(&self.token()), None).await;
        match answer.and_then(|(status, body)| days_answer(status, &body)) {
            Ok(days) => {
                dom::log(&format!(
                    "lufs days {instance} {target}: {} days",
                    days.len()
                ));
                Some(days)
            }
            Err(why) => {
                dom::log(&format!("lufs days {instance} {target} not read: {why}"));
                None
            }
        }
    }
}
