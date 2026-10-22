#ifndef SHELL_STOP_H
#define SHELL_STOP_H

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef struct ShellChild ShellChild;
struct ShellChild {
    pid_t pid;
    void *context;
    int (*still_running)(const ShellChild *child);
    void (*reap)(const ShellChild *child);
};

static volatile sig_atomic_t shell_stop_write_fd = -1;

static void shell_stop_on_signal(int signo) {
    int saved = errno;
    int fd = (int)shell_stop_write_fd;
    unsigned char byte = 1;
    (void)signo;
    if (fd >= 0) {
        for (;;) {
            ssize_t wrote = write(fd, &byte, 1);
            if (wrote >= 0 || errno != EINTR) break;
        }
    }
    errno = saved;
}

static int shell_stop_fail(int read_fd, int write_fd) {
    int saved = errno;
    struct sigaction action;
    memset(&action, 0, sizeof action);
    action.sa_handler = SIG_DFL;
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGINT, &action, NULL);
    if (read_fd >= 0) close(read_fd);
    if (write_fd >= 0) close(write_fd);
    shell_stop_write_fd = -1;
    errno = saved;
    return -1;
}

static int shell_stop_prepare(int fd) {
    int flags = fcntl(fd, F_GETFD);
    if (flags < 0) return -1;
    if (fcntl(fd, F_SETFD, flags | FD_CLOEXEC) < 0) return -1;
    flags = fcntl(fd, F_GETFL);
    if (flags < 0) return -1;
    if (fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0) return -1;
    return 0;
}

static int shell_stop_arm(int *quit_fd) {
    int ends[2];
    struct sigaction action;
    if (quit_fd == NULL || shell_stop_write_fd != -1) {
        errno = EALREADY;
        return -1;
    }
    if (pipe(ends) < 0) return shell_stop_fail(-1, -1);
    if (shell_stop_prepare(ends[0]) < 0 || shell_stop_prepare(ends[1]) < 0) {
        return shell_stop_fail(ends[0], ends[1]);
    }
    shell_stop_write_fd = (sig_atomic_t)ends[1];
    memset(&action, 0, sizeof action);
    action.sa_handler = shell_stop_on_signal;
    action.sa_flags = SA_RESTART;
    // Caught rather than ignored, because an ignored signal is inherited across
    // exec and the child's polite SIGTERM becomes the one-second SIGKILL. A caught
    // signal resets to the default disposition on exec.
    if (sigemptyset(&action.sa_mask) < 0 ||
        sigaction(SIGTERM, &action, NULL) < 0 ||
        sigaction(SIGINT, &action, NULL) < 0) {
        return shell_stop_fail(ends[0], ends[1]);
    }
    *quit_fd = ends[0];
    return 0;
}

static int shell_stop_take_quit(int quit_fd) {
    char buffer[64];
    int saw = 0;
    for (;;) {
        ssize_t got = read(quit_fd, buffer, sizeof buffer);
        if (got > 0) {
            saw = 1;
            continue;
        }
        if (got < 0 && errno == EINTR) continue;
        return saw;
    }
}

static int shell_stop_deadline_reached(const struct timespec *deadline) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return 1;
    if (now.tv_sec != deadline->tv_sec) return now.tv_sec > deadline->tv_sec;
    return now.tv_nsec >= deadline->tv_nsec;
}

static void shell_stop_signal(pid_t pid, int signo) {
    if (kill(pid, signo) < 0) errno = 0;
}

static void shell_stop_child(const ShellChild *child) {
    struct timespec deadline;
    if (child == NULL || child->pid <= 0) return;
    if (child->still_running == NULL || child->reap == NULL) abort();
    if (!child->still_running(child)) {
        child->reap(child);
        return;
    }
    shell_stop_signal(child->pid, SIGTERM);
    if (clock_gettime(CLOCK_MONOTONIC, &deadline) != 0) {
        shell_stop_signal(child->pid, SIGKILL);
        child->reap(child);
        return;
    }
    deadline.tv_sec += 1;
    while (child->still_running(child)) {
        if (shell_stop_deadline_reached(&deadline)) break;
        usleep(20000);
        if (shell_stop_deadline_reached(&deadline)) break;
    }
    if (child->still_running(child)) shell_stop_signal(child->pid, SIGKILL);
    child->reap(child);
}

#endif
