/*
 * amr-hook: Fast C relay for Claude Code hooks.
 * Replaces fork+exec of 1.1MB Rust binary with ~16KB C binary.
 *
 * approve-mcp, stop, post-build: self-contained (no socket)
 * ambient, subagent-start: relay to MCP server via Unix socket
 *
 * Stack-only (no malloc). Links only libSystem.
 * Build: cc -O2 -arch arm64 -o amr-hook amr-hook.c
 */

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <string.h>
#include <fcntl.h>
#include <time.h>
#include <stdio.h>
#include <stdlib.h>

#define STDIN_CAP 65536
#define SOCK_CAP  65536

static char stdin_buf[STDIN_CAP];
static char sock_buf[SOCK_CAP];

/* Read all of stdin. Returns byte count. */
static int read_stdin(void) {
    int total = 0, n;
    while (total < STDIN_CAP - 1) {
        n = (int)read(0, stdin_buf + total, STDIN_CAP - 1 - total);
        if (n <= 0) break;
        total += n;
    }
    stdin_buf[total] = '\0';
    return total;
}

/* Connect to ~/.amaranthine/hook.sock, send msg, read response into sock_buf.
 * Returns response length, or -1 on failure. */
static int sock_relay(const char *msg, int len) {
    const char *home = getenv("HOME");
    if (!home) return -1;

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    int plen = snprintf(addr.sun_path, sizeof(addr.sun_path),
                        "%s/.amaranthine/hook.sock", home);
    if (plen >= (int)sizeof(addr.sun_path)) return -1;

    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return -1;

    /* 50ms timeouts — hook budget is 5s, this is plenty */
    struct timeval tv = { .tv_sec = 0, .tv_usec = 50000 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));

    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }

    /* Send request + newline (protocol delimiter) */
    write(fd, msg, len);
    write(fd, "\n", 1);

    /* Read response until newline or EOF */
    int total = 0;
    while (total < SOCK_CAP - 1) {
        int r = (int)read(fd, sock_buf + total, SOCK_CAP - 1 - total);
        if (r <= 0) break;
        total += r;
        if (memchr(sock_buf + total - r, '\n', r)) break;
    }
    close(fd);
    sock_buf[total] = '\0';

    /* Trim trailing whitespace */
    while (total > 0 && sock_buf[total - 1] <= ' ')
        sock_buf[--total] = '\0';

    return total;
}

/* approve-mcp: static permission allow response */
static void approve_mcp(void) {
    static const char r[] =
        "{\"hookSpecificOutput\":{\"hookEventName\":\"PermissionRequest\","
        "\"decision\":{\"behavior\":\"allow\"}}}\n";
    write(1, r, sizeof(r) - 1);
}

/* stop: debounced reminder (120s window) */
static void hook_stop(void) {
    static const char stamp[] = "/tmp/amaranthine-hook-stop.last";
    time_t now = time(NULL);

    /* Check debounce */
    int fd = open(stamp, O_RDONLY);
    if (fd >= 0) {
        char tb[32];
        int n = (int)read(fd, tb, sizeof(tb) - 1);
        close(fd);
        if (n > 0) {
            tb[n] = '\0';
            long last = 0;
            for (int i = 0; i < n && tb[i] >= '0' && tb[i] <= '9'; i++)
                last = last * 10 + (tb[i] - '0');
            if (now - last < 120) return;
        }
    }

    /* Write new timestamp */
    char tb[32];
    int tlen = snprintf(tb, sizeof(tb), "%ld", (long)now);
    fd = open(stamp, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) { write(fd, tb, tlen); close(fd); }

    static const char r[] =
        "{\"hookSpecificOutput\":{\"additionalContext\":"
        "\"STOPPING: Store any non-obvious findings in amaranthine before ending.\"}}\n";
    write(1, r, sizeof(r) - 1);
}

/* post-build: detect build commands in stdin */
static void post_build(void) {
    int n = read_stdin();
    if (n == 0) return;

    int hit = (strstr(stdin_buf, "xcodebuild") && strstr(stdin_buf, "build"))
           || strstr(stdin_buf, "cargo build")
           || strstr(stdin_buf, "swift build")
           || strstr(stdin_buf, "swiftc ");
    if (!hit) return;

    static const char r[] =
        "{\"systemMessage\":\"BUILD COMPLETED. If the build failed with a "
        "non-obvious error, store the root cause in amaranthine (topic: "
        "build-gotchas). If it succeeded after fixing an issue, store what "
        "fixed it.\"}\n";
    write(1, r, sizeof(r) - 1);
}

/* ambient: splice op field into stdin JSON, relay to socket.
 * stdin: {"tool_name":"Read","tool_input":{...}}
 * sends: {"op":"hook_ambient","tool_name":"Read","tool_input":{...}} */
static void ambient(void) {
    int n = read_stdin();
    if (n == 0) return;

    /* Find opening brace */
    char *brace = memchr(stdin_buf, '{', n);
    if (!brace) return;

    static const char pfx[] = "{\"op\":\"hook_ambient\",";
    int pfx_len = sizeof(pfx) - 1;
    char *rest = brace + 1;
    int rest_len = n - (int)(rest - stdin_buf);

    /* Trim trailing whitespace */
    while (rest_len > 0 && (unsigned char)rest[rest_len - 1] <= ' ')
        rest_len--;

    if (pfx_len + rest_len >= SOCK_CAP) return;

    /* Build request in sock_buf, then send (write completes before read overwrites) */
    memcpy(sock_buf, pfx, pfx_len);
    memcpy(sock_buf + pfx_len, rest, rest_len);
    int msg_len = pfx_len + rest_len;

    int resp = sock_relay(sock_buf, msg_len);
    if (resp > 0) {
        write(1, sock_buf, resp);
        write(1, "\n", 1);
    }
}

/* subagent-start: request topic list from socket */
static void subagent_start(void) {
    static const char req[] = "{\"op\":\"hook_ambient\",\"type\":\"subagent-start\"}";
    int resp = sock_relay(req, sizeof(req) - 1);
    if (resp > 0) {
        write(1, sock_buf, resp);
        write(1, "\n", 1);
    }
}

int main(int argc, char **argv) {
    if (argc < 2) return 1;
    const char *cmd = argv[1];

    /* Dispatch on first two chars for speed */
    if (cmd[0] == 'a' && cmd[1] == 'p') approve_mcp();       /* approve-mcp */
    else if (cmd[0] == 'a' && cmd[1] == 'm') ambient();       /* ambient */
    else if (cmd[0] == 'p') post_build();                      /* post-build */
    else if (cmd[0] == 's' && cmd[1] == 't') hook_stop();     /* stop */
    else if (cmd[0] == 's' && cmd[1] == 'u') subagent_start(); /* subagent-start */

    return 0;
}
