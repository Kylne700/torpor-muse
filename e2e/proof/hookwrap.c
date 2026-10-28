// hookwrap: runs one Muse hook command and logs how long it took and what it printed, then hands the hook's
// stdin, stdout, stderr and exit code on unchanged. It is measurement only: the adapter and the engine are not touched.
//
//   hookwrap [-k] LOGFILE EVENT 'shell command of the hook'
//
// Per run it appends two JSON lines to LOGFILE: {"ph":"start",...} before the command starts and {"ph":"end",...} after
// it ends (a start without an end is a run that was killed, for example by Muse's timeout). Times are nanoseconds on
// the monotonic clock around the child only: fork, `sh -c`, the command, its exit. Our own startup, the log
// writes and the copy of the output are outside the interval. "out" holds the hook's stdout (at most 32 KiB; the
// exact byte count is out_bytes).
//
// -k: if META_API_KEY is not in our environment, take it from the nearest ancestor process that has it (Muse
// Code does not hand it to hooks) and give it to the command. Never logged, never in argv, never written anywhere.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define OUT_KEEP 32768

static long long now(clockid_t c) {
  struct timespec t;
  clock_gettime(c, &t);
  return (long long)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void put(int fd, const char *s, size_t n) {
  while (n > 0) {
    ssize_t w = write(fd, s, n);
    if (w < 0) {
      if (errno == EINTR) continue;
      return;
    }
    s += w;
    n -= (size_t)w;
  }
}

// The ancestor's environment holds NAME=value pairs separated by NULs. Returns a malloc'd value or NULL.
static char *ancestor_key(void) {
  long pid = getppid();
  for (int hop = 0; hop < 8 && pid > 1; hop++) {
    char p[64];
    snprintf(p, sizeof p, "/proc/%ld/environ", pid);
    FILE *f = fopen(p, "r");
    if (f) {
      static char buf[1 << 20];
      size_t n = fread(buf, 1, sizeof buf - 1, f);
      fclose(f);
      buf[n] = 0;
      for (size_t i = 0; i < n;) {
        size_t len = strlen(buf + i);
        if (strncmp(buf + i, "META_API_KEY=", 13) == 0 && len > 13) return strdup(buf + i + 13);
        i += len + 1;
      }
    }
    snprintf(p, sizeof p, "/proc/%ld/stat", pid);
    f = fopen(p, "r");
    if (!f) return NULL;
    char line[1024];
    char *ok = fgets(line, sizeof line, f);
    fclose(f);
    if (!ok) return NULL;
    char *rp = strrchr(line, ')'); // "pid (comm) S ppid ..." and comm may hold spaces or parentheses
    long next = 0;
    if (!rp || sscanf(rp + 1, " %*c %ld", &next) != 1) return NULL;
    pid = next;
  }
  return NULL;
}

int main(int argc, char **argv) {
  int want_key = 0, a = 1;
  if (a < argc && strcmp(argv[a], "-k") == 0) want_key = 1, a++;
  if (argc - a != 3) {
    fprintf(stderr, "usage: hookwrap [-k] LOGFILE EVENT 'command'\n");
    return 2;
  }
  const char *logf = argv[a], *event = argv[a + 1], *cmd = argv[a + 2];
  int had_key = getenv("META_API_KEY") != NULL, got_key = 0;
  if (want_key && !had_key) {
    char *k = ancestor_key();
    if (k) {
      setenv("META_API_KEY", k, 1);
      memset(k, 0, strlen(k));
      free(k);
      got_key = 1;
    }
  }
  int lg = open(logf, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0600);
  int mem = memfd_create("hookout", MFD_CLOEXEC);
  if (mem < 0) return 70;
  char head[512];
  int hn;
  long long wall = now(CLOCK_REALTIME);
  long long t0 = now(CLOCK_MONOTONIC);
  hn = snprintf(head, sizeof head, "{\"ph\":\"start\",\"ev\":\"%s\",\"pid\":%ld,\"wall_ns\":%lld,\"t0\":%lld,\"had_key\":%d,\"got_key\":%d}\n",
                event, (long)getpid(), wall, t0, had_key, got_key);
  if (hn > (int)sizeof head - 1) hn = (int)sizeof head - 1;
  if (lg >= 0) put(lg, head, (size_t)hn);
  pid_t kid = fork();
  if (kid < 0) return 71;
  if (kid == 0) {
    dup2(mem, 1);
    execl("/bin/sh", "sh", "-c", cmd, (char *)NULL);
    _exit(127);
  }
  int st = 0;
  while (waitpid(kid, &st, 0) < 0 && errno == EINTR) {}
  long long t1 = now(CLOCK_MONOTONIC);
  int code = WIFEXITED(st) ? WEXITSTATUS(st) : 128 + WTERMSIG(st);
  off_t size = lseek(mem, 0, SEEK_END);
  lseek(mem, 0, SEEK_SET);
  char *out = malloc((size_t)size + 1);
  size_t got = 0;
  while (out && got < (size_t)size) {
    ssize_t r = read(mem, out + got, (size_t)size - got);
    if (r <= 0) break;
    got += (size_t)r;
  }
  put(1, out ? out : "", got);
  if (lg >= 0) {
    hn = snprintf(head, sizeof head, "{\"ph\":\"end\",\"ev\":\"%s\",\"pid\":%ld,\"wall_ns\":%lld,\"t0\":%lld,\"t1\":%lld,\"ns\":%lld,\"exit\":%d,\"sig\":%d,\"out_bytes\":%zu,\"out\":",
                  event, (long)getpid(), wall, t0, t1, t1 - t0, code, WIFSIGNALED(st) ? WTERMSIG(st) : 0, got);
    if (hn > (int)sizeof head - 1) hn = (int)sizeof head - 1;
    // One write per line keeps the line whole when hooks of one event overlap.
    size_t keep = got < OUT_KEEP ? got : OUT_KEEP;
    char *line = malloc((size_t)hn + keep * 6 + 8), *w;
    if (line) {
      memcpy(line, head, (size_t)hn);
      w = line + hn;
      *w++ = '"';
      for (size_t i = 0; i < keep; i++) {
        unsigned char c = (unsigned char)out[i];
        if (c == '"' || c == '\\') *w++ = '\\', *w++ = (char)c;
        else if (c < 0x20) w += sprintf(w, "\\u%04x", c);
        else *w++ = (char)c;
      }
      *w++ = '"', *w++ = '}', *w++ = '\n';
      put(lg, line, (size_t)(w - line));
      free(line);
    }
  }
  return code;
}
