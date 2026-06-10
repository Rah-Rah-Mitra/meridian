# meridian-bench report

- version: 0.0.1 · kernel: `6.18.33+rpt-rpi-2712` · page size: 16384 · cores: 4
- compiled suites: fusion, embed, thermal, ann, lexical, disk, rerank, anon
- build: release-bench (thin LTO, target-cpu=cortex-a76); product profile is fat LTO — gates re-validated under it at Phase-1 exit

## ann — **GATE FAIL** (411.3s)

Gate: some swept ef reaches recall@10 ≥0.95 at p99 <40ms AND 16K mmap view ok

| metric | value |
|---|---|
| build_rss_delta_mb | 483 |
| build_vectors_per_sec | 2579.0 |
| index_size | 1000000 |
| memory_usage_mb | 506 |
| mmap_view_smoke | "pass" |
| recall_at_10_ef_128 | 0.867 |
| recall_at_10_ef_192 | 0.884 |
| recall_at_10_ef_256 | 0.884 |
| recall_at_10_ef_64 | 0.858 |
| search_p50_ms_ef_128 | 0.667 |
| search_p50_ms_ef_192 | 0.895 |
| search_p50_ms_ef_256 | 1.277 |
| search_p50_ms_ef_64 | 0.466 |
| search_p99_ms_ef_128 | 1.486 |
| search_p99_ms_ef_192 | 1.86 |
| search_p99_ms_ef_256 | 2.506 |
| search_p99_ms_ef_64 | 0.959 |
| vectors | 1000000 |


