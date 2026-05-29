#include <wayland-client.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct probe_state {
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    int success;
};

static void registry_handle_global(void *data, struct wl_registry *registry,
                                   uint32_t id, const char *interface, uint32_t version) {
    struct probe_state *state = data;
    if (strcmp(interface, "wl_compositor") == 0) {
        state->compositor = wl_registry_bind(registry, id, &wl_compositor_interface, version < 4 ? version : 4);
    } else if (strcmp(interface, "wl_shm") == 0) {
        state->shm = wl_registry_bind(registry, id, &wl_shm_interface, 1);
    }
}

static void registry_handle_global_remove(void *data, struct wl_registry *registry, uint32_t id) {
    // No-op
}

static const struct wl_registry_listener registry_listener = {
    registry_handle_global,
    registry_handle_global_remove
};

int run_libwayland_probe(int fd) {
    struct wl_display *display = wl_display_connect_to_fd(fd);
    if (!display) {
        fprintf(stderr, "failed to connect to wayland fd %d\n", fd);
        return -1;
    }

    struct probe_state state = { .compositor = NULL, .shm = NULL, .success = 0 };
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &state);

    // Initial roundtrip to get globals
    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip 1 failed\n");
        wl_display_disconnect(display);
        return -2;
    }

    if (!state.compositor || !state.shm) {
        fprintf(stderr, "missing core globals: compositor=%p, shm=%p\n", state.compositor, state.shm);
        wl_display_disconnect(display);
        return -3;
    }

    // Try to create a surface
    struct wl_surface *surface = wl_compositor_create_surface(state.compositor);
    if (!surface) {
        fprintf(stderr, "failed to create surface\n");
        wl_display_disconnect(display);
        return -4;
    }

    // Final roundtrip to ensure requests are processed
    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip 2 failed\n");
        wl_display_disconnect(display);
        return -5;
    }

    wl_surface_destroy(surface);
    wl_compositor_destroy(state.compositor);
    wl_shm_destroy(state.shm);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);

    return 0;
}
