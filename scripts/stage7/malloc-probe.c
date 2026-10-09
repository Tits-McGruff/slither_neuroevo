/* Linux/glibc diagnostic preload: observe allocations without changing production code.
 * Compile with: gcc -shared -fPIC -O2 -Wall -Wextra -Werror -pthread malloc-probe.c -o malloc-probe.so
 * Set SLITHER_MALLOC_PROBE_REPORT to a new task-owned JSONL file and LD_PRELOAD
 * to this library only for a diagnostic process. Optional trimming changes the
 * observed allocator state and must never be presented as an acceptance run.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <malloc.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

/* One exclusively created report and one monotonic origin per observed process. */
static FILE *report;
static struct timespec started;
static double trim_seconds;

/* Elapsed monotonic seconds since this preload initialized. */
static double elapsed(void) {
  struct timespec now;
  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return 0;
  return (double)(now.tv_sec - started.tv_sec) + (double)(now.tv_nsec - started.tv_nsec) / 1e9;
}

/* Read resident pages without allocating a stdio buffer or invoking Node. */
static size_t resident_bytes(void) {
  char buffer[128];
  int fd = open("/proc/self/statm", O_RDONLY | O_CLOEXEC);
  if (fd < 0) return 0;
  ssize_t length = read(fd, buffer, sizeof(buffer) - 1);
  close(fd);
  if (length <= 0) return 0;
  buffer[length] = '\0';
  size_t virtual_pages, resident_pages;
  if (sscanf(buffer, "%zu %zu", &virtual_pages, &resident_pages) != 2) return 0;
  return resident_pages * (size_t)sysconf(_SC_PAGESIZE);
}

/* Fixed scalar samples; glibc arena bytes and mmap allocations remain separate. */
static void sample(const char *event) {
  struct mallinfo2 info = mallinfo2();
  struct timespec now;
  clock_gettime(CLOCK_REALTIME, &now);
  double unix_millis = (double)now.tv_sec * 1000 + (double)now.tv_nsec / 1e6;
  fprintf(report, "{\"pid\":%ld,\"wallSeconds\":%.6f,\"unixMillis\":%.3f,\"event\":\"%s\","
    "\"residentBytes\":%zu,"
    "\"arenaBytes\":%zu,\"allocatedArenaBytes\":%zu,\"freeArenaBytes\":%zu,"
    "\"mappedAllocationBytes\":%zu,\"mappedAllocationCount\":%zu,"
    "\"freeChunkCount\":%zu,\"topReleasableBytes\":%zu}\n",
    (long)getpid(), elapsed(), unix_millis, event, resident_bytes(), info.arena, info.uordblks, info.fordblks,
    info.hblkhd, info.hblks, info.ordblks, info.keepcost);
  fflush(report);
}

/* An optional one-time trim gives causal attribution; normal samples are read-only. */
static void *observe(void *unused) {
  (void)unused;
  int trimmed = 0;
  for (;;) {
    sample("sample");
    if (!trimmed && trim_seconds > 0 && elapsed() >= trim_seconds) {
      sample("beforeTrim");
      int released = malloc_trim(0);
      fprintf(report, "{\"pid\":%ld,\"wallSeconds\":%.6f,\"event\":\"trim\",\"released\":%d}\n",
        (long)getpid(), elapsed(), released);
      sample("afterTrim");
      trimmed = 1;
    }
    struct timespec remaining = { .tv_sec = 5, .tv_nsec = 0 };
    while (nanosleep(&remaining, &remaining) != 0 && errno == EINTR) {}
  }
  return NULL;
}

/* No output or thread is created unless a new report path was explicitly supplied. */
__attribute__((constructor)) static void initialize(void) {
  const char *path = getenv("SLITHER_MALLOC_PROBE_REPORT");
  if (!path || !*path || clock_gettime(CLOCK_MONOTONIC, &started) != 0) return;
  const char *trim = getenv("SLITHER_MALLOC_PROBE_TRIM_SECONDS");
  if (trim && *trim) {
    char *end = NULL;
    trim_seconds = strtod(trim, &end);
    if (*end || !(trim_seconds > 0 && trim_seconds <= 7200)) return;
  }
  int fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
  if (fd < 0) return;
  report = fdopen(fd, "w");
  if (!report) { close(fd); return; }
  pthread_t thread;
  if (pthread_create(&thread, NULL, observe, NULL) != 0) { fclose(report); return; }
  pthread_detach(thread);
}
