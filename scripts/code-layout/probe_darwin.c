/*
 * Temporary training probe: reset the main image's __text residency, then
 * record the pages faulted back in at a signal or at normal process exit.
 *
 * Loaded only through DYLD_INSERT_LIBRARIES. It changes no shipped artifact.
 */
#define _DARWIN_C_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <mach-o/dyld.h>
#include <mach-o/getsect.h>
#include <mach-o/loader.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#define PATH_CAPACITY 4096

static uintptr_t code_address;
static size_t code_length;
static size_t page_size;
static char *resident;
static char sample_path[PATH_CAPACITY];
static char exit_path[PATH_CAPACITY];

static void fail(const char *message) {
    fprintf(stderr, "dim-layout probe: %s: %s\n", message, strerror(errno));
    _exit(120);
}

static size_t number(char *out, size_t value) {
    char digits[32];
    size_t count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value);
    for (size_t index = 0; index < count; ++index)
        out[index] = digits[count - index - 1];
    return count;
}

static void append(char *buffer, size_t *used, const char *text, size_t length) {
    if (*used + length > 65536)
        return;
    memcpy(buffer + *used, text, length);
    *used += length;
}

static void write_sample(const char *path) {
    if (!path[0])
        return;
    size_t count = (code_length + page_size - 1) / page_size;
    if (mincore((void *)code_address, code_length, resident)) {
        fprintf(stderr, "dim-layout probe: mincore: %s\n", strerror(errno));
        return;
    }
    int descriptor = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (descriptor < 0) {
        fprintf(stderr, "dim-layout probe: open %s: %s\n", path, strerror(errno));
        return;
    }
    char buffer[65536];
    size_t used = 0;
    append(buffer, &used, "pagesize ", 9);
    used += number(buffer + used, page_size);
    append(buffer, &used, "\n", 1);
    for (size_t index = 0; index < count; ++index) {
        if (!(resident[index] & 1))
            continue;
        used += number(buffer + used, index * page_size);
        append(buffer, &used, "\n", 1);
        if (used > sizeof(buffer) - 64) {
            if (write(descriptor, buffer, used) != (ssize_t)used)
                _exit(121);
            used = 0;
        }
    }
    if (used && write(descriptor, buffer, used) != (ssize_t)used)
        _exit(121);
    close(descriptor);
}

static void on_signal(int number_seen) {
    (void)number_seen;
    write_sample(sample_path);
}

static void on_exit(void) {
    write_sample(exit_path);
}

__attribute__((constructor)) static void setup(void) {
    const char *sample = getenv("DIM_LAYOUT_SAMPLE");
    const char *at_exit = getenv("DIM_LAYOUT_EXIT_SAMPLE");
    if (sample && strlen(sample) < sizeof(sample_path))
        strcpy(sample_path, sample);
    if (at_exit && strlen(at_exit) < sizeof(exit_path))
        strcpy(exit_path, at_exit);
    unsetenv("DIM_LAYOUT_SAMPLE");
    unsetenv("DIM_LAYOUT_EXIT_SAMPLE");
    unsetenv("DYLD_INSERT_LIBRARIES");

    const struct mach_header_64 *header =
        (const struct mach_header_64 *)_dyld_get_image_header(0);
    if (!header || header->magic != MH_MAGIC_64) {
        errno = ENOEXEC;
        fail("not a 64-bit Mach-O main image");
    }
    const struct section_64 *text =
        getsectbynamefromheader_64(header, "__TEXT", "__text");
    if (!text) {
        errno = ENOEXEC;
        fail("no __TEXT,__text section");
    }
    page_size = (size_t)sysconf(_SC_PAGESIZE);
    if (page_size != 4096 && page_size != 16384) {
        errno = EINVAL;
        fail("unexpected page size");
    }
    uintptr_t text_address = (uintptr_t)(text->addr + _dyld_get_image_vmaddr_slide(0));
    uintptr_t base = text_address & ~(page_size - 1);
    size_t length = ((text_address + text->size + page_size - 1) & ~(page_size - 1)) - base;
    if (!length) {
        errno = ENOEXEC;
        fail("empty __text section");
    }
    const char *image = _dyld_get_image_name(0);
    int descriptor = open(image, O_RDONLY);
    if (descriptor < 0)
        fail("cannot open the main image");
    off_t offset = (off_t)text->offset - (off_t)(text_address - base);
    if (offset < 0 || offset % (off_t)page_size) {
        errno = EINVAL;
        fail("__text file offset is not page-congruent");
    }
    void *mapped = mmap((void *)base, length, PROT_READ | PROT_EXEC,
                        MAP_PRIVATE | MAP_FIXED, descriptor, offset);
    int saved = errno;
    close(descriptor);
    if (mapped == MAP_FAILED) {
        errno = saved;
        fail("cannot reset __text residency");
    }
    resident = malloc((length + page_size - 1) / page_size);
    if (!resident)
        fail("cannot allocate the residency vector");
    code_address = base;
    code_length = length;

    if (sample_path[0]) {
        struct sigaction action;
        memset(&action, 0, sizeof(action));
        action.sa_handler = on_signal;
        sigemptyset(&action.sa_mask);
        if (sigaction(SIGUSR1, &action, NULL))
            fail("cannot install SIGUSR1");
    }
    if (exit_path[0] && atexit(on_exit))
        fail("cannot register the exit sample");
}
