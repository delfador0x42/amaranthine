/* bench.c — measure amaranthine query latency via C FFI */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "../include/amaranthine.h"

static double now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e9 + ts.tv_nsec;
}

int main(int argc, char** argv) {
    char index_path[1024];
    if (argc > 1) {
        snprintf(index_path, sizeof(index_path), "%s", argv[1]);
    } else {
        snprintf(index_path, sizeof(index_path), "%s/.amaranthine/index.bin", getenv("HOME"));
    }

    AmrIndex* idx = amr_open(index_path);
    if (!idx) { fprintf(stderr, "failed to open %s\n", index_path); return 1; }

    char* info = amr_info(idx);
    if (info) { printf("%s\n", info); amr_free_str(info); }

    int N = 10000;
    double start, elapsed;

    /* --- Standard API benchmarks --- */

    const char* queries[] = {
        "ExecPolicy enforcement", "DNS tunneling",
        "baseline anomaly detection", "persistence scanner",
        "network event bridge",
    };
    int nq = sizeof(queries) / sizeof(queries[0]);

    for (int i = 0; i < 100; i++) {
        char* r = amr_search(idx, queries[i % nq], 5);
        if (r) amr_free_str(r);
    }

    start = now_ns();
    for (int i = 0; i < N; i++) {
        char* r = amr_search(idx, queries[i % nq], 5);
        if (r) amr_free_str(r);
    }
    elapsed = now_ns() - start;
    printf("amr_search (multi):  %4.0f ns/query\n", elapsed / N);

    start = now_ns();
    for (int i = 0; i < N; i++) {
        char* r = amr_search(idx, "ExecPolicy", 5);
        if (r) amr_free_str(r);
    }
    elapsed = now_ns() - start;
    printf("amr_search (single): %4.0f ns/query\n", elapsed / N);

    /* --- Zero-alloc API benchmarks --- */

    /* Pre-hash terms once */
    uint64_t h_exec = amr_hash("execpolicy");
    uint64_t h_enforce = amr_hash("enforcement");
    uint64_t h_dns = amr_hash("dns");
    uint64_t h_tunnel = amr_hash("tunneling");

    uint64_t multi_hashes[][2] = {
        {h_exec, h_enforce}, {h_dns, h_tunnel},
    };
    AmrResult results[5];

    /* warm up */
    for (int i = 0; i < 100; i++) {
        amr_search_raw(idx, &h_exec, 1, results, 5);
    }

    /* single pre-hashed term */
    start = now_ns();
    for (int i = 0; i < N; i++) {
        amr_search_raw(idx, &h_exec, 1, results, 5);
    }
    elapsed = now_ns() - start;
    printf("search_raw (1 hash): %4.0f ns/query\n", elapsed / N);

    /* two pre-hashed terms */
    start = now_ns();
    for (int i = 0; i < N; i++) {
        amr_search_raw(idx, multi_hashes[i % 2], 2, results, 5);
    }
    elapsed = now_ns() - start;
    printf("search_raw (2 hash): %4.0f ns/query\n", elapsed / N);

    /* staleness check */
    start = now_ns();
    for (int i = 0; i < N; i++) {
        amr_is_stale(idx);
    }
    elapsed = now_ns() - start;
    printf("stale_check:         %4.0f ns/call\n", elapsed / N);

    /* snippet lookup */
    start = now_ns();
    for (int i = 0; i < N; i++) {
        uint32_t len;
        amr_snippet(idx, results[0].entry_id, &len);
    }
    elapsed = now_ns() - start;
    printf("snippet:             %4.0f ns/call\n", elapsed / N);

    /* verify results make sense */
    uint32_t nr = amr_search_raw(idx, &h_exec, 1, results, 5);
    printf("\n%u results for 'execpolicy':\n", nr);
    for (uint32_t i = 0; i < nr; i++) {
        uint32_t len;
        const uint8_t* snip = amr_snippet(idx, results[i].entry_id, &len);
        if (snip) printf("  [%u] score=%u  %.*s\n",
            results[i].entry_id, results[i].score_x1000, len, snip);
    }

    amr_close(idx);
    return 0;
}
