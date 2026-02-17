/* amaranthine.h — C FFI for direct in-process query */

#ifndef AMARANTHINE_H
#define AMARANTHINE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct AmrIndex AmrIndex;

/* Result from zero-alloc search */
typedef struct {
    uint16_t entry_id;
    uint32_t score_x1000;
} AmrResult;

/* --- Standard API (~1μs, convenient) --- */

AmrIndex* amr_open(const char* index_path);
char*     amr_search(const AmrIndex* idx, const char* query, uint32_t limit);
char*     amr_info(const AmrIndex* idx);
int       amr_is_stale(const AmrIndex* idx);
int       amr_reload(AmrIndex* idx);
void      amr_free_str(char* s);
void      amr_close(AmrIndex* idx);

/* --- Zero-alloc API (~100-200ns, no heap allocation) --- */

/* Hash a term. Caller caches the result for repeated queries. */
uint64_t  amr_hash(const char* term);

/* Search with pre-hashed terms. Writes into caller's buffer.
   Returns number of results written. Zero heap allocation. */
uint32_t  amr_search_raw(AmrIndex* idx, const uint64_t* hashes, uint32_t nhashes,
                          AmrResult* out, uint32_t limit);

/* Get snippet for entry_id. Returns ptr into index data + length.
   Valid until amr_reload/amr_close. Do NOT free the pointer. */
const uint8_t* amr_snippet(const AmrIndex* idx, uint16_t entry_id,
                            uint32_t* out_len);

#ifdef __cplusplus
}
#endif

#endif
