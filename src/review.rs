//! Portable review metadata and a safe, page-referenced CSV summary.
use crate::annotations::Annotation;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub author: String,
    pub created: i64,
    pub resolved: bool,
    pub replies: Vec<Reply>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub author: String,
    pub created: i64,
    pub text: String,
}
pub fn timestamp() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}
pub fn csv(marks: &[Annotation]) -> String {
    fn cell(s: &str) -> String {
        let guarded = if s.trim_start().starts_with(['=', '+', '-', '@']) {
            format!("'{s}")
        } else {
            s.to_owned()
        };
        format!("\"{}\"", guarded.replace('"', "\"\""))
    }
    let mut out =
        "Page,Type,Object,Author,Status,Text,Reply author,Reply time (UTC),Reply\r\n".to_owned();
    for a in marks {
        let r = &a.object.review;
        let row = |reply: Option<&Reply>| {
            [
                (a.page + 1).to_string(),
                a.kind.label().into(),
                a.object.name.clone(),
                r.author.clone(),
                if r.resolved { "Resolved" } else { "Open" }.into(),
                a.kind.text().unwrap_or("").into(),
                reply.map_or(String::new(), |r| r.author.clone()),
                reply
                    .and_then(|r| time::OffsetDateTime::from_unix_timestamp(r.created).ok())
                    .map_or(String::new(), |t| t.to_string()),
                reply.map_or(String::new(), |r| r.text.clone()),
            ]
            .iter()
            .map(|s| cell(s))
            .collect::<Vec<_>>()
            .join(",")
                + "\r\n"
        };
        if r.replies.is_empty() {
            out.push_str(&row(None));
        } else {
            for reply in &r.replies {
                out.push_str(&row(Some(reply)));
            }
        }
    }
    out
}
