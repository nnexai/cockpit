/* Native acceptance only. Refuse any compositor outside a matching, private
 * /tmp/cnative-* fixture before connecting or sending input.
 * Build: cc -O2 -Wall -Wextra -Werror private_wayland_input.c -o private_wayland_input -lwayland-client -lxkbcommon
 */
#define _GNU_SOURCE
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/input-event-codes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static const struct wl_message keyboard_requests[] = {
    {"keymap", "uhu", NULL}, {"key", "uuu", NULL}, {"modifiers", "uuuu", NULL}, {"destroy", "", NULL},
};
static const struct wl_interface keyboard_interface = {"zwp_virtual_keyboard_v1", 1, 4, keyboard_requests, 0, NULL};
static const struct wl_interface *keyboard_create_types[] = {&wl_seat_interface, &keyboard_interface};
static const struct wl_message keyboard_manager_requests[] = { {"create_virtual_keyboard", "on", keyboard_create_types} };
static const struct wl_interface keyboard_manager_interface = {"zwp_virtual_keyboard_manager_v1", 1, 1, keyboard_manager_requests, 0, NULL};
static const struct wl_message pointer_requests[] = {
    {"motion", "uff", NULL}, {"motion_absolute", "uuuuu", NULL}, {"button", "uuu", NULL},
    {"axis", "uuf", NULL}, {"frame", "", NULL}, {"axis_source", "u", NULL},
    {"axis_stop", "uu", NULL}, {"axis_discrete", "uufi", NULL}, {"destroy", "", NULL},
};
static const struct wl_interface pointer_interface = {"zwlr_virtual_pointer_v1", 1, 9, pointer_requests, 0, NULL};
static const struct wl_interface *pointer_create_types[] = {&wl_seat_interface, &pointer_interface};
static const struct wl_message pointer_manager_requests[] = { {"create_virtual_pointer", "?on", pointer_create_types}, {"destroy", "", NULL} };
static const struct wl_interface pointer_manager_interface = {"zwlr_virtual_pointer_manager_v1", 1, 2, pointer_manager_requests, 0, NULL};

struct devices { struct wl_seat *seat; struct wl_proxy *keyboard_manager, *pointer_manager; };
static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    struct devices *dev = data;
    if (!strcmp(interface, "wl_seat")) dev->seat = wl_registry_bind(registry, name, &wl_seat_interface, version < 5 ? version : 5);
    else if (!strcmp(interface, "zwp_virtual_keyboard_manager_v1")) dev->keyboard_manager = wl_registry_bind(registry, name, &keyboard_manager_interface, 1);
    else if (!strcmp(interface, "zwlr_virtual_pointer_manager_v1")) dev->pointer_manager = wl_registry_bind(registry, name, &pointer_manager_interface, 1);
}
static void global_remove(void *data, struct wl_registry *registry, uint32_t name) { (void)data; (void)registry; (void)name; }
static const struct wl_registry_listener listener = {global, global_remove};
static uint32_t timestamp_ms(void) { struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now); return (uint32_t)(now.tv_sec * 1000u + now.tv_nsec / 1000000u); }
static void fail(const char *reason) { fprintf(stderr, "%s\n", reason); exit(2); }
static bool owned_socket(const char *root) {
    if (strncmp(root, "/tmp/cnative-", 13)) return false;
    struct stat st;
    if (lstat(root, &st) || !S_ISDIR(st.st_mode) || st.st_uid != getuid() || (st.st_mode & 0077)) return false;
    char log[1024]; if (snprintf(log, sizeof log, "%s/compositor.log", root) >= (int)sizeof log) return false;
    if (lstat(log, &st) || !S_ISREG(st.st_mode) || st.st_uid != getuid()) return false;
    FILE *file = fopen(log, "r"); if (!file) return false;
    const char *wayland = getenv("WAYLAND_DISPLAY"), *niri = getenv("NIRI_SOCKET");
    bool display_match = false, ipc_match = false;
    char *line = NULL; size_t capacity = 0;
    while (getline(&line, &capacity, file) != -1) {
        const char *marker = strstr(line, "listening on Wayland socket: ");
        if (marker && wayland) { marker += strlen("listening on Wayland socket: "); display_match |= strncmp(marker, wayland, strlen(wayland)) == 0 && (marker[strlen(wayland)] == '\n' || marker[strlen(wayland)] == '\r'); }
        marker = strstr(line, "IPC listening on: ");
        if (marker && niri) { marker += strlen("IPC listening on: "); ipc_match |= strncmp(marker, niri, strlen(niri)) == 0 && (marker[strlen(niri)] == '\n' || marker[strlen(niri)] == '\r'); }
    }
    free(line); fclose(file);
    return display_match && ipc_match && !strstr(wayland, "wayland-1");
}
static uint32_t number(const char *text) { char *end = NULL; errno = 0; unsigned long value = strtoul(text, &end, 10); if (errno || !end || *end || value > UINT32_MAX) fail("invalid nonnegative numeric input"); return (uint32_t)value; }
static void present(struct wl_display *display) { if (wl_display_roundtrip(display) < 0) fail("private compositor rejected input"); }
int main(int argc, char **argv) {
    if (argc < 4 || !owned_socket(argv[1])) fail("refusing input outside the matching owned /tmp/cnative-* compositor");
    struct wl_display *display = wl_display_connect(NULL); if (!display) fail("private compositor socket unavailable");
    struct devices dev = {0}; struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, &dev); present(display);
    if (!dev.seat) fail("private compositor seat missing");
    if (!strcmp(argv[2], "key")) {
        if (argc != 4 || !dev.keyboard_manager) fail("key <evdev-keycode> or virtual keyboard unsupported");
        struct xkb_context *ctx = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
        struct xkb_keymap *map = xkb_keymap_new_from_names(ctx, NULL, XKB_KEYMAP_COMPILE_NO_FLAGS);
        if (!map) fail("cannot create virtual keyboard keymap");
        char *text = xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1);
        int fd = memfd_create("cockpit-private-keymap", 0); if (fd < 0 || !text) fail("cannot create virtual keyboard memfd");
        size_t len = strlen(text) + 1;
        if (write(fd, text, len) != (ssize_t)len) fail("virtual keyboard keymap write failed");
        struct wl_proxy *keyboard = wl_proxy_marshal_constructor_versioned(dev.keyboard_manager, 0, &keyboard_interface, 1, dev.seat, NULL);
        if (!keyboard) fail("virtual keyboard creation failed");
        wl_proxy_marshal(keyboard, 0, 1u, fd, (uint32_t)len); close(fd); present(display);
        uint32_t code = number(argv[3]);
        wl_proxy_marshal(keyboard, 1, timestamp_ms(), code, 1u); present(display);
        usleep(50000);
        wl_proxy_marshal(keyboard, 1, timestamp_ms(), code, 0u); present(display);
        wl_proxy_marshal(keyboard, 3); wl_proxy_destroy(keyboard);
        free(text); xkb_keymap_unref(map); xkb_context_unref(ctx);
    } else if (!strcmp(argv[2], "move") || !strcmp(argv[2], "click") || !strcmp(argv[2], "drag")) {
        bool drag = !strcmp(argv[2], "drag"), move = !strcmp(argv[2], "move");
        if (argc != (drag ? 9 : 7) || !dev.pointer_manager) fail("move/click <x> <y> <width> <height> or drag <x> <y> <end-x> <end-y> <width> <height>");
        uint32_t x = number(argv[3]), y = number(argv[4]);
        uint32_t end_x = drag ? number(argv[5]) : x, end_y = drag ? number(argv[6]) : y;
        uint32_t width = number(argv[drag ? 7 : 5]), height = number(argv[drag ? 8 : 6]);
        if (!width || !height || x >= width || y >= height || end_x >= width || end_y >= height) fail("pointer coordinates outside owned output");
        struct wl_proxy *pointer = wl_proxy_marshal_constructor_versioned(dev.pointer_manager, 0, &pointer_interface, 1, dev.seat, NULL);
        if (!pointer) fail("virtual pointer creation failed");
        wl_proxy_marshal(pointer, 1, timestamp_ms(), x, y, width, height);
        wl_proxy_marshal(pointer, 4); present(display); usleep(30000);
        if (!move) {
            wl_proxy_marshal(pointer, 2, timestamp_ms(), BTN_LEFT, 1u);
            wl_proxy_marshal(pointer, 4); present(display); usleep(50000);
            if (drag) { wl_proxy_marshal(pointer, 1, timestamp_ms(), end_x, end_y, width, height); wl_proxy_marshal(pointer, 4); present(display); usleep(40000); }
            wl_proxy_marshal(pointer, 2, timestamp_ms(), BTN_LEFT, 0u);
            wl_proxy_marshal(pointer, 4); present(display);
        }
        wl_proxy_marshal(pointer, 8); wl_proxy_destroy(pointer);
    } else fail("expected key, move, click, or drag");
    wl_display_disconnect(display);
    puts("private compositor input sequence completed");
    return 0;
}
