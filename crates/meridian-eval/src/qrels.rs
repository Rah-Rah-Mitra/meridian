//! TREC-style qrels: `qid 0 doc-id grade` per line (doc-id = URL here).
//! Queries file: `qid<TAB>query text` per line.

use crate::metrics::Judgments;
use std::collections::HashMap;
use std::path::Path;

pub struct EvalSet {
    /// qid → query text
    pub queries: Vec<(String, String)>,
    /// qid → judgments
    pub qrels: HashMap<String, Judgments>,
}

pub fn load(queries_path: &Path, qrels_path: &Path) -> std::io::Result<EvalSet> {
    let mut queries = Vec::new();
    for line in std::fs::read_to_string(queries_path)?.lines() {
        if let Some((qid, text)) = line.split_once('\t') {
            if !qid.is_empty() && !text.trim().is_empty() {
                queries.push((qid.to_owned(), text.trim().to_owned()));
            }
        }
    }
    let mut qrels: HashMap<String, Judgments> = HashMap::new();
    for line in std::fs::read_to_string(qrels_path)?.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 4 {
            if let Ok(grade) = parts[3].parse::<u32>() {
                qrels
                    .entry(parts[0].to_owned())
                    .or_default()
                    .insert(parts[2].to_owned(), grade);
            }
        }
    }
    Ok(EvalSet { queries, qrels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip() {
        let dir = std::env::temp_dir().join(format!("meridian-qrels-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("q.tsv"),
            "q1\tborrow checker rust\nq2\tmetric system\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("qrels.txt"),
            "q1 0 https://a.example/1 3\nq2 0 https://b.example/2 3\nq2 0 https://b.example/3 1\n",
        )
        .unwrap();
        let set = load(&dir.join("q.tsv"), &dir.join("qrels.txt")).unwrap();
        assert_eq!(set.queries.len(), 2);
        assert_eq!(set.qrels["q2"].len(), 2);
        assert_eq!(set.qrels["q2"]["https://b.example/3"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
