#include "xdg_shell_probe.h"
#include <stddef.h>

static const struct wl_interface *xdg_shell_types[] = {
    NULL,
    NULL,
    NULL,
    NULL,
    &xdg_surface_interface,
    &wl_surface_interface,
    &xdg_toplevel_interface,
};

static const struct wl_message xdg_wm_base_requests[] = {
    { "destroy", "", xdg_shell_types + 0 },
    { "create_positioner", "n", xdg_shell_types + 0 },
    { "get_xdg_surface", "no", xdg_shell_types + 4 },
    { "pong", "u", xdg_shell_types + 0 },
};

static const struct wl_message xdg_wm_base_events[] = {
    { "ping", "u", xdg_shell_types + 0 },
};

const struct wl_interface xdg_wm_base_interface = {
    "xdg_wm_base", 6,
    4, xdg_wm_base_requests,
    1, xdg_wm_base_events,
};

static const struct wl_message xdg_surface_requests[] = {
    { "destroy", "", xdg_shell_types + 0 },
    { "get_toplevel", "n", xdg_shell_types + 6 },
    { "get_popup", "n?oo", xdg_shell_types + 0 },
    { "set_window_geometry", "iiii", xdg_shell_types + 0 },
    { "ack_configure", "u", xdg_shell_types + 0 },
};

static const struct wl_message xdg_surface_events[] = {
    { "configure", "u", xdg_shell_types + 0 },
};

const struct wl_interface xdg_surface_interface = {
    "xdg_surface", 6,
    5, xdg_surface_requests,
    1, xdg_surface_events,
};

static const struct wl_message xdg_toplevel_requests[] = {
    { "destroy", "", xdg_shell_types + 0 },
    { "set_parent", "?o", xdg_shell_types + 0 },
    { "set_title", "s", xdg_shell_types + 0 },
    { "set_app_id", "s", xdg_shell_types + 0 },
};

static const struct wl_message xdg_toplevel_events[] = {
    { "configure", "iia", xdg_shell_types + 0 },
    { "close", "", xdg_shell_types + 0 },
};

const struct wl_interface xdg_toplevel_interface = {
    "xdg_toplevel", 6,
    4, xdg_toplevel_requests,
    2, xdg_toplevel_events,
};
