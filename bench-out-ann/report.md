# meridian-bench report

- version: 0.0.1 · kernel: `6.18.33+rpt-rpi-2712` · page size: 16384 · cores: 4
- compiled suites: fusion, embed, thermal, ann, lexical, disk, rerank, anon
- build: release-bench (thin LTO, target-cpu=cortex-a76); product profile is fat LTO — gates re-validated under it at Phase-1 exit

## ann — **GATE FAIL** (342.2s)

Gate: p99 <40ms @ ef=64 AND recall@10 ≥0.95 AND 16K mmap view ok

| metric | value |
|---|---|
| build_rss_delta_mb | 483 |
| build_vectors_per_sec | 3130.0 |
| index_size | 1000000 |
| memory_usage_mb | 506 |
| mmap_view_smoke | "pass" |
| recall_at_10 | 0.798 |
| search_p50_ms | 0.413705 |
| search_p99_ms | 0.8911140000000001 |
| vectors | 1000000 |


