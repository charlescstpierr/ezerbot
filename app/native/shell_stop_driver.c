// Parent shaped like the Mac shell. Quit signals arrive on the descriptor from
// shell_stop.h. The child is exec'd after the handlers are installed, so a caught
// signal in the parent becomes the default disposition in the child.
#include "shell_stop.h"

#include <errno.h>
#include <poll.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

typedef struct {
    int reaped;
    int status;
} DriverState;

static int driver_still_running(const ShellChild *child) {
    DriverState *state = child->context;
    if (state->reaped) return 0;
    int status = 0;
    pid_t got = waitpid(child->pid, &status, WNOHANG);
    if (got == 0) return 1;
    if (got == child->pid) {
        state->reaped = 1;
        state->status = status;
        return 0;
    }
    if (got < 0 && errno == ECHILD) {
        state->reaped = 1;
        return 0;
    }
    return 1;
}

static void driver_reap(const ShellChild *child) {
    DriverState *state = child->context;
    if (state->reaped) return;
    int status = 0;
    pid_t got;
    do {
        got = waitpid(child->pid, &status, 0);
    } while (got < 0 && errno == EINTR);
    state->reaped = 1;
    if (got == child->pid) state->status = status;
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "child-ignore") == 0) {
        signal(SIGTERM, SIG_IGN);
        int fd = argc > 2 ? atoi(argv[2]) : -1;
        if (fd >= 0) {
            char go = 1;
            if (write(fd, &go, 1) != 1) _exit(2);
            close(fd);
        }
        for (;;) pause();
    }
    int quit_fd = -1;
    if (shell_stop_arm(&quit_fd) != 0) return 2;
    int ignore = argc > 1 && strcmp(argv[1], "ignore") == 0;
    char *sleep_argv[] = {"/bin/sleep", "60", NULL};
    int note[2] = {-1, -1};
    char note_text[16];
    char *ignore_argv[] = {argv[0], "child-ignore", note_text, NULL};
    pid_t child = 0;
    int spawned;
    if (ignore) {
        if (pipe(note) != 0) return 2;
        snprintf(note_text, sizeof note_text, "%d", note[1]);
        spawned = posix_spawn(&child, argv[0], NULL, NULL, ignore_argv, environ);
        close(note[1]);
        if (spawned != 0) return 2;
        char go = 0;
        if (read(note[0], &go, 1) != 1) return 2;
        close(note[0]);
    } else {
        spawned = posix_spawn(&child, "/bin/sleep", NULL, NULL, sleep_argv, environ);
        if (spawned != 0) return 2;
    }
    printf("ready %d\n", (int)child);
    fflush(stdout);
    for (;;) {
        struct pollfd slot = {.fd = quit_fd, .events = POLLIN};
        if (poll(&slot, 1, -1) < 0) {
            if (errno == EINTR) continue;
            return 3;
        }
        if (!shell_stop_take_quit(quit_fd)) continue;
        DriverState state = {0};
        ShellChild stop = {
            .pid = child,
            .context = &state,
            .still_running = driver_still_running,
            .reap = driver_reap,
        };
        shell_stop_child(&stop);
        shell_stop_child(&stop);
        const char *how = "other";
        if (WIFSIGNALED(state.status) && WTERMSIG(state.status) == SIGTERM) how = "term";
        if (WIFSIGNALED(state.status) && WTERMSIG(state.status) == SIGKILL) how = "kill";
        printf("stopped %s\n", how);
        return 0;
    }
}
