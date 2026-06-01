#include "xdg_shell_probe.h"
#include <wayland-client.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <fcntl.h>
#include <errno.h>

struct probe_state {
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct xdg_wm_base *wm_base;
    struct xdg_surface *xdg_surface;
    struct xdg_toplevel *xdg_toplevel;
    uint32_t last_serial;
    int configured;
};

static void wm_base_ping(void *data, struct xdg_wm_base *wm_base, uint32_t serial) {
    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_base_listener = {
    wm_base_ping,
};

static void xdg_surface_configure(void *data, struct xdg_surface *xdg_surface, uint32_t serial) {
    struct probe_state *state = data;
    state->last_serial = serial;
    state->configured = 1;
    xdg_surface_ack_configure(xdg_surface, serial);
}

static const struct xdg_surface_listener xdg_surface_listener = {
    xdg_surface_configure,
};

static void xdg_toplevel_configure(void *data, struct xdg_toplevel *xdg_toplevel,
                                  int32_t width, int32_t height, struct wl_array *states) {
    // No-op for probe
}

static void xdg_toplevel_close(void *data, struct xdg_toplevel *xdg_toplevel) {
    // No-op for probe
}

static const struct xdg_toplevel_listener xdg_toplevel_listener = {
    xdg_toplevel_configure,
    xdg_toplevel_close,
};

static void registry_handle_global(void *data, struct wl_registry *registry,
                                   uint32_t id, const char *interface, uint32_t version) {
    struct probe_state *state = data;
    if (strcmp(interface, "wl_compositor") == 0) {
        state->compositor = wl_registry_bind(registry, id, &wl_compositor_interface, version < 4 ? version : 4);
    } else if (strcmp(interface, "wl_shm") == 0) {
        state->shm = wl_registry_bind(registry, id, &wl_shm_interface, 1);
    } else if (strcmp(interface, "xdg_wm_base") == 0) {
        state->wm_base = wl_registry_bind(registry, id, &xdg_wm_base_interface, 1);
        xdg_wm_base_add_listener(state->wm_base, &wm_base_listener, state);
    }
}

static void registry_handle_global_remove(void *data, struct wl_registry *registry, uint32_t id) {
    // No-op
}

static const struct wl_registry_listener registry_listener = {
    registry_handle_global,
    registry_handle_global_remove
};

static int create_shm_fd(size_t size) {
    int fd = -1;
#ifdef MFD_CLOEXEC
    fd = memfd_create("tuff-xwin-shm", MFD_CLOEXEC);
#endif
    if (fd < 0) {
        char name[] = "/tmp/tuff-xwin-shm-XXXXXX";
        fd = mkstemp(name);
        if (fd >= 0) {
            unlink(name);
        }
    }
    if (fd >= 0) {
        if (ftruncate(fd, size) < 0) {
            close(fd);
            return -1;
        }
    }
    return fd;
}

int run_libwayland_probe(int fd) {
    struct wl_display *display = wl_display_connect_to_fd(fd);
    if (!display) {
        fprintf(stderr, "failed to connect to wayland fd %d\n", fd);
        return -1;
    }

    struct probe_state state = { .compositor = NULL, .shm = NULL, .wm_base = NULL, .xdg_surface = NULL, .xdg_toplevel = NULL, .last_serial = 0, .configured = 0 };
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &state);

    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip 1 failed\n");
        wl_display_disconnect(display);
        return -2;
    }

    if (!state.compositor || !state.shm || !state.wm_base) {
        fprintf(stderr, "missing core globals: compositor=%p, shm=%p, wm_base=%p\n", state.compositor, state.shm, state.wm_base);
        wl_display_disconnect(display);
        return -3;
    }

    struct wl_surface *surface = wl_compositor_create_surface(state.compositor);
    
    // XDG shell setup
    state.xdg_surface = xdg_wm_base_get_xdg_surface(state.wm_base, surface);
    xdg_surface_add_listener(state.xdg_surface, &xdg_surface_listener, &state);
    state.xdg_toplevel = xdg_surface_get_toplevel(state.xdg_surface);
    xdg_toplevel_add_listener(state.xdg_toplevel, &xdg_toplevel_listener, &state);
    xdg_toplevel_set_title(state.xdg_toplevel, "Harness Probe");
    xdg_toplevel_set_app_id(state.xdg_toplevel, "tuff.xwin.probe");
    
    wl_surface_commit(surface);

    // Roundtrip to get configure events
    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip 2 failed\n");
        wl_display_disconnect(display);
        return -5;
    }

    if (!state.configured) {
        fprintf(stderr, "xdg_surface was not configured\n");
        wl_display_disconnect(display);
        return -7;
    }

    // Create SHM pool and buffer
    size_t shm_size = 64 * 64 * 4;
    int shm_fd = create_shm_fd(shm_size);
    if (shm_fd < 0) {
        fprintf(stderr, "failed to create shm fd\n");
        wl_display_disconnect(display);
        return -6;
    }
    
    struct wl_shm_pool *pool = wl_shm_create_pool(state.shm, shm_fd, shm_size);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(pool, 0, 64, 64, 64 * 4, WL_SHM_FORMAT_ARGB8888);
    close(shm_fd);

    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage(surface, 0, 0, 64, 64);
    wl_surface_commit(surface);

    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip 3 failed\n");
        wl_display_disconnect(display);
        return -5;
    }

    wl_buffer_destroy(buffer);
    wl_shm_pool_destroy(pool);
    xdg_toplevel_destroy(state.xdg_toplevel);
    xdg_surface_destroy(state.xdg_surface);
    wl_surface_destroy(surface);
    xdg_wm_base_destroy(state.wm_base);
    wl_compositor_destroy(state.compositor);
    wl_shm_destroy(state.shm);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);

    return 0;
}
