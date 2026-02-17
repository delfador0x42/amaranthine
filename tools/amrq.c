/* amrq — fast amaranthine query. Links libamaranthine statically.
 * Process startup (~5ms) + query (~1μs) = ~5ms total.
 *
 * Flags:
 *   -f    Full entry mode: fetch complete entries from topic files
 *   -n N  Limit results (default: 5) */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../include/amaranthine.h"

/* Read file into malloc'd buffer. Caller frees. */
static char* read_file(const char* path, size_t* out_len) {
    FILE* f = fopen(path, "r");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    char* buf = malloc(len + 1);
    if (!buf) { fclose(f); return NULL; }
    *out_len = fread(buf, 1, len, f);
    buf[*out_len] = '\0';
    fclose(f);
    return buf;
}

/* Find entry in topic file matching "## YYYY-MM-DD HH:MM" prefix.
 * Prints from that header through end of entry. */
static void print_full_entry(const char* amr_dir, const char* topic,
                              const char* timestamp) {
    char fpath[1024];
    snprintf(fpath, sizeof(fpath), "%s/%s.md", amr_dir, topic);
    size_t len;
    char* content = read_file(fpath, &len);
    if (!content) return;

    /* Build search pattern: "## YYYY-MM-DD HH:MM" */
    char pattern[32];
    snprintf(pattern, sizeof(pattern), "## %.16s", timestamp);
    size_t plen = strlen(pattern);

    char* p = content;
    while ((p = strstr(p, pattern)) != NULL) {
        /* Verify it's at line start */
        if (p != content && *(p - 1) != '\n') { p++; continue; }

        /* Find end of entry (next "## YYYY-" or EOF) */
        char* end = p + 1;
        while (*end) {
            if (*end == '\n' && end[1] == '#' && end[2] == '#' && end[3] == ' '
                && end[4] >= '0' && end[4] <= '9' && end[7] == '-') {
                end++; /* include the newline */
                break;
            }
            end++;
        }
        printf("[%s]\n", topic);
        fwrite(p, 1, end - p, stdout);
        if (*(end - 1) != '\n') putchar('\n');
        putchar('\n');
        break;
    }
    free(content);
}

int main(int argc, char** argv) {
    int full = 0, limit = 5;
    const char* query = NULL;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-f") == 0) { full = 1; }
        else if (strcmp(argv[i], "-n") == 0 && i + 1 < argc) { limit = atoi(argv[++i]); }
        else if (!query) { query = argv[i]; }
        else { limit = atoi(argv[i]); } /* backwards compat: amrq "query" 3 */
    }

    if (!query) {
        fprintf(stderr, "usage: amrq [-f] [-n limit] <query>\n");
        return 1;
    }

    /* Resolve amaranthine directory */
    char amr_dir[1024];
    const char* dir_env = getenv("AMARANTHINE_DIR");
    if (dir_env)
        snprintf(amr_dir, sizeof(amr_dir), "%s", dir_env);
    else
        snprintf(amr_dir, sizeof(amr_dir), "%s/.amaranthine", getenv("HOME"));

    char index_path[1024];
    snprintf(index_path, sizeof(index_path), "%s/index.bin", amr_dir);

    AmrIndex* idx = amr_open(index_path);
    if (!idx) { fprintf(stderr, "no index at %s\n", index_path); return 1; }

    char* result = amr_search(idx, query, limit);
    if (!result) { amr_close(idx); return 0; }

    if (!full) {
        printf("%s", result);
    } else {
        /* Parse snippets: "  [topic] YYYY-MM-DD HH:MM text..." */
        char* line = result;
        while (*line) {
            char* nl = strchr(line, '\n');
            if (!nl) nl = line + strlen(line);

            char* ob = strchr(line, '[');
            char* cb = ob ? strchr(ob, ']') : NULL;
            if (ob && cb && cb - ob < 200) {
                char topic[256];
                int tlen = (int)(cb - ob - 1);
                memcpy(topic, ob + 1, tlen);
                topic[tlen] = '\0';

                /* Timestamp starts after "] " */
                const char* ts = cb + 2;
                if (ts + 16 <= nl) {
                    print_full_entry(amr_dir, topic, ts);
                }
            }
            line = *nl ? nl + 1 : nl;
        }
    }

    amr_free_str(result);
    amr_close(idx);
    return 0;
}
