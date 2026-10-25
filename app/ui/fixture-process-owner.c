#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#ifdef __linux__
#include <sys/prctl.h>
#endif
#include <time.h>
#include <unistd.h>

#define CONTROL_FD 3
#define EVENTS_FD 4
#define BOUND_MS 3000
#define TICK_MS 20

enum reservation { RESERVED, RETIRED, ANCHOR_REAPED };
enum payload { ABSENT, HELD, RELEASED, TERMINATING, PAYLOAD_REAPED };
enum mode { ACCEPTING, STOPPING, FINISHING };

struct owner {
    pid_t anchor;
    pid_t target;
    enum reservation reservation;
    enum payload payload;
    enum mode mode;
    bool anchor_ready;
    bool go_seen;
    bool stopped;
    bool events_ok;
    bool control_open;
    bool target_setup;
    int lease;
    int anchor_ack;
    int target_ack;
    int target_release;
    int result;
    long long deadline;
    long long startup_deadline;
    char command[32];
    size_t command_length;
    unsigned char target_record[sizeof(int)];
    size_t target_record_length;
};

static volatile sig_atomic_t interrupted;

static void on_signal(int signal_number) {
    interrupted = signal_number;
}

static long long now_ms(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value) < 0) _exit(125);
    return (long long)value.tv_sec * 1000 + value.tv_nsec / 1000000;
}

static void close_fd(int *fd) {
    if (*fd >= 0) {
        close(*fd);
        *fd = -1;
    }
}

static int configure_fd(int fd, bool nonblocking) {
    int flags = fcntl(fd, F_GETFD);
    if (flags < 0 || fcntl(fd, F_SETFD, flags | FD_CLOEXEC) < 0) return -1;
    if (nonblocking) {
        flags = fcntl(fd, F_GETFL);
        if (flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0) return -1;
    }
    return 0;
}

static int private_pipe(int descriptors[2]) {
    if (pipe(descriptors) < 0) return -1;
    if (configure_fd(descriptors[0], false) < 0 ||
        configure_fd(descriptors[1], false) < 0) {
        int saved = errno;
        close(descriptors[0]);
        close(descriptors[1]);
        errno = saved;
        return -1;
    }
    return 0;
}

static void close_except(int first, int second, bool stdio, int limit) {
    for (int fd = stdio ? 3 : 0; fd < limit; fd++) {
        if (fd != first && fd != second) close(fd);
    }
}

static void reset_child_signals(void) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = SIG_DFL;
    sigemptyset(&action.sa_mask);
    const int signals[] = { SIGCHLD, SIGPIPE, SIGTERM, SIGINT, SIGHUP };
    for (size_t i = 0; i < sizeof(signals) / sizeof(signals[0]); i++) {
        if (sigaction(signals[i], &action, NULL) < 0) _exit(126);
    }
}

static void begin_deadline(struct owner *owner) {
    if (owner->deadline == 0) owner->deadline = now_ms() + BOUND_MS;
}

static bool event(struct owner *owner, const char *name, long pid, int value) {
    if (!owner->events_ok) return false;
    char record[192];
    int length = snprintf(record, sizeof(record),
                          "{\"event\":\"%s\",\"pid\":%ld,\"value\":%d}\n",
                          name, pid, value);
    if (length <= 0 || (size_t)length >= sizeof(record) ||
        write(EVENTS_FD, record, (size_t)length) != length) {
        owner->events_ok = false;
        owner->result = 1;
        return false;
    }
    return true;
}

static void stop_group(struct owner *owner) {
    begin_deadline(owner);
    if (owner->mode == ACCEPTING) owner->mode = STOPPING;
    if (owner->stopped) return;
    owner->stopped = true;
    close_fd(&owner->target_release);
    close_fd(&owner->anchor_ack);
    int failure = 0;
    if (owner->payload == HELD || owner->payload == RELEASED || owner->payload == TERMINATING) {
        if (kill(owner->target, SIGKILL) < 0 && errno != ESRCH) failure = errno;
    }
    if (owner->reservation == RESERVED && owner->anchor > 1) {
        if (kill(-owner->anchor, SIGKILL) < 0 && errno != ESRCH) failure = errno;
    }
    if (failure != 0) owner->result = 1;
    event(owner, "stopped", (long)owner->anchor, failure);
}

static void finish(struct owner *owner) {
    stop_group(owner);
    owner->mode = FINISHING;
    /* No path may use the negative group ID after this transition. */
    if (owner->reservation == RESERVED) owner->reservation = RETIRED;
    close_fd(&owner->lease);
    close_fd(&owner->anchor_ack);
}

static void fail(struct owner *owner, const char *name, int error) {
    owner->result = 1;
    event(owner, name, 0, error);
    finish(owner);
}

static void reap_target(struct owner *owner) {
    if (owner->payload == ABSENT || owner->payload == PAYLOAD_REAPED) return;
    int status;
    pid_t waited = waitpid(owner->target, &status, WNOHANG);
    if (waited == owner->target) {
        owner->payload = PAYLOAD_REAPED;
        close_fd(&owner->target_release);
        int value = WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
        if (WIFSIGNALED(status)) event(owner, "target-signal", (long)owner->target, WTERMSIG(status));
        event(owner, "exited", (long)owner->target, value);
    } else if (waited < 0 && errno != EINTR) {
        /* ECHILD means the ownership assumption failed. Never signal this PID. */
        owner->payload = PAYLOAD_REAPED;
        fail(owner, "wait-failed", errno);
    }
}

static void command(struct owner *owner, const char *line) {
    if (strcmp(line, "go") == 0 && !owner->go_seen && owner->mode == ACCEPTING) {
        owner->go_seen = true;
    } else if (strcmp(line, "stop") == 0) {
        stop_group(owner);
    } else if (strcmp(line, "finish") == 0) {
        finish(owner);
    } else if (strcmp(line, "kill-target") == 0 && owner->go_seen) {
        reap_target(owner);
        if (owner->payload == PAYLOAD_REAPED) {
            event(owner, "target-terminal", (long)owner->target, 0);
        } else if (owner->payload == HELD || owner->payload == RELEASED || owner->payload == TERMINATING) {
            close_fd(&owner->target_release);
            owner->payload = TERMINATING;
            if (kill(owner->target, SIGKILL) < 0 && errno != ESRCH)
                fail(owner, "target-signal-failed", errno);
        } else {
            fail(owner, "protocol-error", 0);
        }
    } else if (strcmp(line, "kill-anchor") == 0 && owner->reservation == RESERVED && owner->anchor > 1) {
        int failure = kill(owner->anchor, SIGKILL) == 0 ? 0 : errno;
        if (failure != 0 && failure != ESRCH) {
            fail(owner, "anchor-signal-failed", failure);
        } else {
            event(owner, "anchor-killed", (long)owner->anchor, failure);
        }
    } else {
        fail(owner, "protocol-error", 0);
    }
}

static void read_control(struct owner *owner) {
    if (!owner->control_open || owner->mode == FINISHING) return;
    /* Bound a command flood. The sender cannot defer the cleanup deadline. */
    size_t total = 0;
    for (;;) {
        char input[256];
        ssize_t count = read(CONTROL_FD, input, sizeof(input));
        if (count == 0) {
            owner->control_open = false;
            if (owner->command_length != 0) fail(owner, "partial-command", 0);
            else finish(owner);
            return;
        }
        if (count < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) return;
            if (errno == EINTR) return;
            fail(owner, "control-failed", errno);
            return;
        }
        total += (size_t)count;
        if (total > 4096) {
            fail(owner, "control-overflow", 0);
            return;
        }
        for (ssize_t i = 0; i < count; i++) {
            unsigned char byte = (unsigned char)input[i];
            if (byte == '\n') {
                owner->command[owner->command_length] = '\0';
                command(owner, owner->command);
                owner->command_length = 0;
                if (owner->mode == FINISHING) return;
            } else if (byte < 'a' || byte > 'z') {
                if (byte != '-') {
                    fail(owner, "protocol-error", 0);
                    return;
                }
                if (owner->command_length == sizeof(owner->command) - 1) {
                    fail(owner, "command-too-long", 0);
                    return;
                }
                owner->command[owner->command_length++] = (char)byte;
            } else {
                if (owner->command_length == sizeof(owner->command) - 1) {
                    fail(owner, "command-too-long", 0);
                    return;
                }
                owner->command[owner->command_length++] = (char)byte;
            }
        }
    }
}

static void anchor_child(int ack, int lease, int limit) {
    reset_child_signals();
    if (setpgid(0, 0) < 0) _exit(126);
    close_except(ack, lease, false, limit);
    char ready = 'R';
    if (write(ack, &ready, 1) != 1) _exit(126);
    close(ack);
    for (;;) {
        char byte;
        ssize_t count = read(lease, &byte, 1);
        if (count < 0 && errno == EINTR) continue;
        /* This group is our own immutable group, including on owner death. */
        kill(0, SIGKILL);
        _exit(126);
    }
}

static void payload_child(pid_t anchor, int ack, int release, int limit, char **argv) {
    reset_child_signals();
    close_except(ack, release, true, limit);
    if (setpgid(0, anchor) < 0) {
        int failure = errno;
        (void)write(ack, &failure, sizeof(failure));
        _exit(126);
    }
    int success = 0;
    if (write(ack, &success, sizeof(success)) != (ssize_t)sizeof(success)) _exit(126);
    char permission;
    ssize_t count;
    do { count = read(release, &permission, 1); } while (count < 0 && errno == EINTR);
    close(release);
    if (count != 1 || permission != 'G') _exit(126);
    execvp(argv[0], argv);
    int failure = errno;
    (void)write(ack, &failure, sizeof(failure));
    _exit(127);
}

static void start_target(struct owner *owner, int limit, char **argv) {
    int ack[2], release[2];
    if (private_pipe(ack) < 0) {
        fail(owner, "pipe-failed", errno);
        return;
    }
    if (private_pipe(release) < 0) {
        int failure = errno;
        close(ack[0]); close(ack[1]);
        fail(owner, "pipe-failed", failure);
        return;
    }
    if (configure_fd(ack[0], true) < 0) {
        int failure = errno;
        close(ack[0]); close(ack[1]); close(release[0]); close(release[1]);
        fail(owner, "fd-failed", failure);
        return;
    }
    const int target_descriptors[] = { ack[0], ack[1], release[0], release[1], owner->lease };
    for (size_t i = 0; i < sizeof(target_descriptors) / sizeof(target_descriptors[0]); i++) {
        if (target_descriptors[i] >= limit) limit = target_descriptors[i] + 1;
    }
    read_control(owner);
    if (owner->mode != ACCEPTING || owner->command_length != 0 || interrupted) {
        close(ack[0]); close(ack[1]); close(release[0]); close(release[1]);
        if (interrupted) finish(owner);
        return;
    }
    pid_t child = fork();
    if (child == 0) payload_child(owner->anchor, ack[1], release[0], limit, argv);
    int failure = errno;
    close(ack[1]); close(release[0]);
    if (child < 0) {
        close(ack[0]); close(release[1]);
        fail(owner, "fork-failed", failure);
        return;
    }
    owner->startup_deadline = now_ms() + BOUND_MS;
    owner->target = child;
    owner->payload = HELD;
    owner->target_ack = ack[0];
    owner->target_release = release[1];
    close(STDIN_FILENO);
    close(STDOUT_FILENO);
    close(STDERR_FILENO);
    if (setpgid(child, owner->anchor) < 0) fail(owner, "target-group-failed", errno);
}

static void read_target_ack(struct owner *owner) {
    if (owner->target_ack < 0) return;
    for (;;) {
        ssize_t count = read(owner->target_ack,
                             owner->target_record + owner->target_record_length,
                             sizeof(int) - owner->target_record_length);
        if (count == 0) {
            close_fd(&owner->target_ack);
            if (owner->mode == ACCEPTING && owner->payload != TERMINATING &&
                (owner->target_record_length != 0 || !owner->target_setup))
                fail(owner, "target-handshake-failed", 0);
            return;
        }
        if (count < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) return;
            fail(owner, "target-handshake-failed", errno);
            close_fd(&owner->target_ack);
            return;
        }
        owner->target_record_length += (size_t)count;
        if (owner->target_record_length != sizeof(int)) continue;
        int value;
        memcpy(&value, owner->target_record, sizeof(value));
        owner->target_record_length = 0;
        if (value != 0) {
            fail(owner, owner->target_setup ? "exec-failed" : "target-group-failed", value);
            close_fd(&owner->target_ack);
            return;
        }
        if (owner->target_setup) {
            fail(owner, "target-handshake-failed", 0);
            close_fd(&owner->target_ack);
            return;
        }
        owner->target_setup = true;
    }
}

static void release_target(struct owner *owner) {
    if (!owner->target_setup || owner->payload != HELD || owner->mode != ACCEPTING) return;
    read_control(owner);
    if (owner->mode != ACCEPTING || owner->command_length != 0 || interrupted) return;
    if (!event(owner, "started", (long)owner->target, 0)) {
        finish(owner);
        return;
    }
    read_control(owner);
    if (owner->mode != ACCEPTING || owner->payload != HELD || interrupted) {
        close_fd(&owner->target_release);
        if (interrupted) finish(owner);
        return;
    }
    if (owner->command_length != 0) {
        fail(owner, "partial-command", 0);
        return;
    }
    char permission = 'G';
    if (write(owner->target_release, &permission, 1) != 1) {
        fail(owner, "release-failed", errno);
        return;
    }
    owner->payload = RELEASED;
    close_fd(&owner->target_release);
}

int main(int argc, char **argv) {
    if (argc < 2) return 2;
#ifdef __linux__
    if (prctl(PR_SET_CHILD_SUBREAPER, 1) < 0) return 2;
#endif
    struct owner owner;
    memset(&owner, 0, sizeof(owner));
    owner.lease = owner.anchor_ack = owner.target_ack = owner.target_release = -1;
    owner.events_ok = owner.control_open = true;
    owner.reservation = RETIRED;
    owner.startup_deadline = now_ms() + BOUND_MS;

    struct sigaction action;
    memset(&action, 0, sizeof(action));
    sigemptyset(&action.sa_mask);
    action.sa_handler = SIG_DFL;
    if (sigaction(SIGCHLD, &action, NULL) < 0) return 2;
    action.sa_handler = SIG_IGN;
    if (sigaction(SIGPIPE, &action, NULL) < 0) return 2;
    action.sa_handler = on_signal;
    if (sigaction(SIGTERM, &action, NULL) < 0 ||
        sigaction(SIGINT, &action, NULL) < 0 ||
        sigaction(SIGHUP, &action, NULL) < 0) return 2;
    sigset_t empty;
    sigemptyset(&empty);
    if (sigprocmask(SIG_SETMASK, &empty, NULL) < 0) return 2;
    if (configure_fd(CONTROL_FD, true) < 0 || configure_fd(EVENTS_FD, true) < 0) return 2;
    if (getsid(0) != getpid()) {
        event(&owner, "session-required", 0, 0);
        return 2;
    }
    int limit = EVENTS_FD + 1;
    int ack[2], lease[2];
    if (private_pipe(ack) < 0) return 2;
    if (private_pipe(lease) < 0) { close(ack[0]); close(ack[1]); return 2; }
    if (configure_fd(ack[0], true) < 0) return 2;
    const int anchor_descriptors[] = { ack[0], ack[1], lease[0], lease[1] };
    for (size_t i = 0; i < sizeof(anchor_descriptors) / sizeof(anchor_descriptors[0]); i++) {
        if (anchor_descriptors[i] >= limit) limit = anchor_descriptors[i] + 1;
    }
    read_control(&owner);
    if (owner.mode != ACCEPTING || interrupted) {
        if (interrupted) owner.result = 1;
        event(&owner, "finished", 0, owner.result);
        return owner.result;
    }
    owner.anchor = fork();
    if (owner.anchor == 0) anchor_child(ack[1], lease[0], limit);
    if (owner.anchor < 0) return 2;
    owner.reservation = RESERVED;
    owner.lease = lease[1];
    owner.anchor_ack = ack[0];
    close(ack[1]); close(lease[0]);
    if (setpgid(owner.anchor, owner.anchor) < 0) fail(&owner, "anchor-group-failed", errno);

    for (;;) {
        if (interrupted) {
            owner.result = 1;
            finish(&owner);
        }
        if (!owner.events_ok) finish(&owner);
        read_control(&owner);
        if (owner.anchor_ack >= 0 && !owner.anchor_ready && owner.mode == ACCEPTING) {
            char ready;
            ssize_t count = read(owner.anchor_ack, &ready, 1);
            if (count == 1 && ready == 'R') {
                close_fd(&owner.anchor_ack);
                owner.anchor_ready = true;
                if (!event(&owner, "ready", (long)owner.anchor, 0)) finish(&owner);
            } else if (count == 0 || count > 0 ||
                       (count < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) {
                fail(&owner, "anchor-handshake-failed", count < 0 ? errno : 0);
            }
        }
        read_target_ack(&owner);
        reap_target(&owner);
        release_target(&owner);
        if (owner.anchor_ready && owner.go_seen && owner.payload == ABSENT &&
            owner.mode == ACCEPTING && owner.events_ok && owner.command_length == 0) start_target(&owner, limit, argv + 1);
        long long current = now_ms();
        if (owner.deadline != 0 && current >= owner.deadline) {
            owner.result = 124;
            event(&owner, "deadline", 0, BOUND_MS);
            finish(&owner);
            _exit(124);
        }
        if (owner.mode == ACCEPTING &&
            (!owner.anchor_ready || (owner.payload == HELD && !owner.target_setup)) &&
            current >= owner.startup_deadline) fail(&owner, "startup-deadline", BOUND_MS);
        if (owner.mode == FINISHING) {
            if (owner.reservation == RETIRED) {
                int status;
                pid_t waited = waitpid(owner.anchor, &status, WNOHANG);
                if (waited == owner.anchor) owner.reservation = ANCHOR_REAPED;
                else if (waited < 0 && errno != EINTR) {
                    owner.reservation = ANCHOR_REAPED;
                    owner.result = 1;
                    event(&owner, "anchor-wait-failed", (long)owner.anchor, errno);
                }
            }
            if (owner.reservation == ANCHOR_REAPED &&
                (owner.payload == ABSENT || owner.payload == PAYLOAD_REAPED)) {
#ifdef __linux__
                /* The anchor is retired before the sole reaper collects adopted descendants. */
                int adopted_status;
                pid_t adopted;
                do { adopted = waitpid(-1, &adopted_status, WNOHANG); } while (adopted > 0);
                if (adopted == 0 || (adopted < 0 && errno == EINTR)) {
                    struct timespec delay = { .tv_sec = 0, .tv_nsec = TICK_MS * 1000000 };
                    nanosleep(&delay, NULL);
                    continue;
                }
                if (errno != ECHILD) {
                    event(&owner, "descendant-wait-failed", 0, errno);
                    return 1;
                }
#endif
                if (now_ms() >= owner.deadline) {
                    event(&owner, "deadline", 0, BOUND_MS);
                    return 124;
                }
                if (!event(&owner, "finished", (long)owner.anchor, owner.result)) return 1;
                return now_ms() >= owner.deadline ? 124 : owner.result;
            }
        }
        struct pollfd descriptors[3];
        descriptors[0].fd = owner.control_open && owner.mode != FINISHING ? CONTROL_FD : -1;
        descriptors[0].events = POLLIN;
        descriptors[1].fd = owner.anchor_ack;
        descriptors[1].events = POLLIN;
        descriptors[2].fd = owner.target_ack;
        descriptors[2].events = POLLIN;
        int timeout = TICK_MS;
        if (owner.deadline != 0 && owner.deadline - current < timeout)
            timeout = (int)(owner.deadline - current);
        if (poll(descriptors, 3, timeout) < 0 && errno != EINTR) fail(&owner, "poll-failed", errno);
    }
}
