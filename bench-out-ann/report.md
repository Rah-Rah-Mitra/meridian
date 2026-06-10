# meridian-bench report

- version: 0.0.1 · kernel: `6.18.33+rpt-rpi-2712` · page size: 16384 · cores: 4
- compiled suites: fusion, embed, thermal, ann, lexical, disk, rerank, anon
- build: release-bench (thin LTO, target-cpu=cortex-a76); product profile is fat LTO — gates re-validated under it at Phase-1 exit

## ann — GATE PASS (428.8s)

Gate: some swept ef reaches recall@10 ≥0.95 at p99 <40ms AND 16K mmap view ok

| metric | value |
|---|---|
| build_rss_delta_mb | 483 |
| build_vectors_per_sec | 2584.0 |
| derived_ef_search | 64 |
| index_size | 1000000 |
| memory_usage_mb | 506 |
| mmap_view_smoke | "pass" |
| recall_at_10_ef_128 | 1.0 |
| recall_at_10_ef_192 | 1.0 |
| recall_at_10_ef_256 | 1.0 |
| recall_at_10_ef_64 | 0.98 |
| search_p50_ms_ef_128 | 0.653 |
| search_p50_ms_ef_192 | 0.866 |
| search_p50_ms_ef_256 | 1.258 |
| search_p50_ms_ef_64 | 0.45 |
| search_p99_ms_ef_128 | 1.447 |
| search_p99_ms_ef_192 | 1.777 |
| search_p99_ms_ef_256 | 2.126 |
| search_p99_ms_ef_64 | 0.948 |
| vectors | 1000000 |

- ef_search re-derived per SPEC §15: 64 (recall ≥0.95 at p99 0.95ms)

