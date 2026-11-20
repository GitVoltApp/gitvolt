//! The last few graph payloads a `graph` read with `since` returned, by version, so the next read
//! naming one gets only what changed (`gitvolt_graph::GraphReply`). Every tab on the repository
//! shares them: a version names one payload, whichever tab, worktree or view it was laid out for.

use gitvolt_graph::{diff_payloads, GraphFull, GraphPayload, GraphReply};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Payloads kept: two tabs on one repository, each a refresh or two behind, still get deltas.
const KEPT: usize = 4;

#[derive(Default)]
pub(crate) struct GraphVersions {
    inner: Mutex<Kept>,
}

#[derive(Default)]
struct Kept {
    last: u32,
    payloads: VecDeque<(u32, Arc<GraphPayload>)>,
}

impl GraphVersions {
    /// `payload` as the next version, and the reply to a read holding `since`: a delta from it
    /// when it's still kept, else the whole payload. Returns the reply as JSON.
    pub(crate) fn reply(&self, payload: GraphPayload, since: u32) -> Result<serde_json::Value, serde_json::Error> {
        let (version, base) = {
            let mut k = self.inner.lock().expect("graph versions poisoned");
            // 0 never names one: the UI's "none yet".
            k.last = k.last.wrapping_add(1).max(1);
            (k.last, k.payloads.iter().find(|(v, _)| *v == since && since != 0).map(|(_, p)| p.clone()))
        };
        let (json, payload) = match base {
            Some(base) => (serde_json::to_value(GraphReply::Delta(diff_payloads(&base, &payload, since, version)))?, payload),
            None => {
                let reply = GraphReply::Full(GraphFull { version, graph: payload });
                let json = serde_json::to_value(&reply)?;
                let GraphReply::Full(full) = reply else { unreachable!() };
                (json, full.graph)
            }
        };
        let mut k = self.inner.lock().expect("graph versions poisoned");
        k.payloads.push_back((version, Arc::new(payload)));
        while k.payloads.len() > KEPT {
            k.payloads.pop_front();
        }
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(rows: usize) -> GraphPayload {
        let row = |i: usize| serde_json::json!({
            "id": format!("{i:040x}"), "kind": "commit", "lane": 0, "color": 0, "segments": [], "summary": format!("c{i}"),
            "bodyFirstLine": "", "authorName": "Mara Quill", "authorEmail": "mara@example.com", "authorTime": 0, "committerTime": 0,
            "parents": [], "mrRefs": [], "wip": null
        });
        let rows = (0..rows).map(|i| serde_json::from_value(row(i)).unwrap()).collect();
        GraphPayload { rows, labels: vec![], max_lanes: 1, pinned_refs: vec![], head: Default::default(), truncated: false, open_worktree: None, worktrees: vec![], in_progress: Default::default(), fork: None, view: None }
    }

    #[test]
    fn a_read_naming_a_kept_version_gets_a_delta_and_any_other_the_whole_payload() {
        let v = GraphVersions::default();
        let first = v.reply(payload(3), 0).unwrap();
        assert_eq!((first["kind"].as_str(), first["version"].as_u64()), (Some("full"), Some(1)));
        assert_eq!(first["graph"]["rows"].as_array().unwrap().len(), 3);
        let second = v.reply(payload(4), 1).unwrap();
        assert_eq!((second["kind"].as_str(), second["version"].as_u64(), second["since"].as_u64()), (Some("delta"), Some(2), Some(1)));
        assert_eq!(second["splices"].as_array().unwrap().len(), 1, "one row more");
        assert_eq!(v.reply(payload(4), 99).unwrap()["kind"], "full", "unknown");
        for _ in 0..KEPT {
            v.reply(payload(4), 0).unwrap();
        }
        assert_eq!(v.reply(payload(4), 2).unwrap()["kind"], "full", "too old");
    }
}
