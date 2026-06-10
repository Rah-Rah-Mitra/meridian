//! Evaluation & benchmarking (SPEC §15): relevance eval harness (nDCG@10, MRR@10,
//! Recall@100 over trec-style qrels), the `meridian-bench` on-device suite, criterion
//! microbenches, and the privacy smoke test driver (canary query/IP leak hunt).
//!
//! Status: Phase-0 scaffold — `meridian-bench` is implemented and RUN ON DEVICE as
//! the Phase-0 exit gate, immediately after planning sign-off.
