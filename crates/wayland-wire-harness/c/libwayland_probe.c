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

    struct probe_state state = { .compositor = NULL, .shm = NULL };
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, &state);

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

    struct wl_surface *surface = wl_compositor_create_surface(state.compositor);
    
    // Create SHM pool and buffer
    size_t size = 64 * 64 * 4;
    int shm_fd = create_shm_fd(size);
    if (shm_fd < 0) {
        fprintf(stderr, "failed to create shm fd\n");
        wl_display_disconnect(display);
        return -6;
    }
    
    struct wl_shm_pool *pool = wl_shm_create_pool(state.shm, shm_fd, size);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(pool, 0, 64, 64, 64 * 4, WL_SHM_FORMAT_ARGB8888);
    close(shm_fd);

    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage(surface, 0, 0, 64, 64);
    wl_surface_commit(surface);

    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip 2 failed\n");
        wl_display_disconnect(display);
        return -5;
    }

    wl_buffer_destroy(buffer);
    wl_shm_pool_destroy(pool);
    wl_surface_destroy(surface);
    wl_compositor_destroy(state.compositor);
    wl_shm_destroy(state.shm);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);

    return 0;
}
