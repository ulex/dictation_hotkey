#!/usr/bin/env python3
"""Minimal headless Wayland compositor for end-to-end testing.

Implements just enough of wl_compositor, wl_shm, wl_seat, wl_output,
xdg_wm_base and zwp_virtual_keyboard_manager_v1 to let a Qt client map a
window and receive keyboard input, and to let a virtual-keyboard client
(typing_linux.py) inject text — mirroring how real compositors
(Sway/KWin) forward virtual keyboard events to the focused client.

The first xdg toplevel that commits gets keyboard focus and keeps it.

Usage: python tests/wayland_compositor.py SOCKET_NAME
Prints READY on stdout once the socket is accepting connections.
"""

import os
import sys
import time

DEBUG = bool(os.environ.get("DICTATION_COMPOSITOR_DEBUG"))


def _dbg(msg):
    if DEBUG:
        print(f"[compositor] {msg}", flush=True)

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from pywayland import ffi, lib
import pywayland.protocol_core  # noqa: F401  (import first: avoids circular import)
from pywayland.dispatcher import Dispatcher
from pywayland.protocol.wayland import (
    WlCallback,
    WlCompositor,
    WlKeyboard,
    WlOutput,
    WlSeat,
    WlSurface,
)
from pywayland.protocol.xdg_shell import XdgSurface, XdgToplevel, XdgWmBase
from pywayland.protocol_core.argument import ArgumentType
from pywayland.protocol_core.message import Message
from pywayland.server import Display

from protocol.virtual_keyboard_unstable_v1 import (
    ZwpVirtualKeyboardManagerV1,
    ZwpVirtualKeyboardV1,
)

# ---------------------------------------------------------------------------
# pywayland server-side patches
#
# pywayland 0.4.18 is client-centric; three things are broken/missing for
# servers and are patched here:
# 1. Resources created in global_bind_func get wl_resource_set_dispatcher
#    with implementation=NULL, but libwayland passes that pointer as `data`
#    to dispatcher_func, which expects the python object handle -> crash.
# 2. Message.c_to_arguments resolves Object/NewId args via the *client*
#    registry, which does not exist in a server process.
# 3. Message.arguments_to_c forgets to assign Array args into the wl_argument
#    union (args_ptr[i].a is never set).
# ---------------------------------------------------------------------------

_resources: dict[int, object] = {}


def _fix_dispatcher(resource):
    lib.wl_resource_set_dispatcher(
        resource._ptr,
        lib.dispatcher_func,
        resource._handle,
        resource._handle,
        lib.resource_destroy_func,
    )
    _resources[int(ffi.cast("uintptr_t", resource._ptr))] = resource
    resource.dispatcher._resource = resource


def _wrap_resource(iface, ptr):
    """Wrap an existing wl_resource (created by libwayland) in a pywayland Resource."""
    key = int(ffi.cast("uintptr_t", ptr))
    if key in _resources:
        return _resources[key]
    cls = iface.resource_class
    obj = cls.__new__(cls)
    obj.version = lib.wl_resource_get_version(ptr)
    obj.dispatcher = Dispatcher(iface.requests, destructor=True)
    obj._ptr = ptr
    obj.id = lib.wl_resource_get_id(ptr)
    obj._handle = ffi.new_handle(obj)
    _fix_dispatcher(obj)

    def _on_destroy(res, k=key):
        _resources.pop(k, None)

    obj.dispatcher.destructor = _on_destroy
    return obj


# Server-side requests carry new_id arguments as raw object ids; the handler
# is expected to create the wl_resource itself. To do that we need the client
# of the resource being dispatched on — track it via Dispatcher.__getitem__,
# which dispatcher_func always calls before c_to_arguments.
_dispatch_state: dict = {"target": None}

_orig_getitem = Dispatcher.__getitem__


def _tracking_getitem(self, opcode_or_name):
    _dispatch_state["target"] = getattr(self, "_resource", None)
    return _orig_getitem(self, opcode_or_name)


Dispatcher.__getitem__ = _tracking_getitem


def _create_new_resource(iface, object_id: int):
    """Create a wl_resource for a client-sent new_id (server side)."""
    target = _dispatch_state["target"]
    assert target is not None, "new_id arg without dispatch target"
    client_ptr = lib.wl_resource_get_client(target._ptr)
    cls = iface.resource_class
    obj = cls(client_ptr, target.version, object_id)
    _fix_dispatcher(obj)

    key = int(ffi.cast("uintptr_t", obj._ptr))

    def _on_destroy(res, k=key):
        _resources.pop(k, None)

    obj.dispatcher.destructor = _on_destroy
    return obj


_orig_c_to_arguments = Message.c_to_arguments


def _server_c_to_arguments(self, args_ptr):
    from pywayland.protocol.wayland.wl_registry import WlRegistry

    if WlRegistry.registry:  # a client Display exists in-process: original path
        return _orig_c_to_arguments(self, args_ptr)
    args = []
    for i, argument in enumerate(self.arguments):
        arg_ptr = args_ptr[i]
        t = argument.argument_type
        if t == ArgumentType.Int:
            args.append(arg_ptr.i)
        elif t == ArgumentType.Uint:
            args.append(arg_ptr.u)
        elif t == ArgumentType.Fixed:
            args.append(lib.wl_fixed_to_double(arg_ptr.f))
        elif t == ArgumentType.FileDescriptor:
            args.append(arg_ptr.h)
        elif t == ArgumentType.String:
            args.append(None if arg_ptr.s == ffi.NULL else ffi.string(arg_ptr.s).decode())
        elif t == ArgumentType.Object:
            if arg_ptr.o == ffi.NULL:
                args.append(None)
            else:
                ptr = ffi.cast("struct wl_resource *", arg_ptr.o)
                args.append(_wrap_resource(argument.interface, ptr))
        elif t == ArgumentType.NewId:
            args.append(_create_new_resource(argument.interface, arg_ptr.n))
        elif t == ArgumentType.Array:
            args.append(ffi.buffer(arg_ptr.a.data, arg_ptr.a.size)[:])
        else:
            raise Exception(f"Bad argument: {argument}")
    return args


Message.c_to_arguments = _server_c_to_arguments

def _fixed_arguments_to_c(self, *args):
    """Server-side arguments_to_c with a working Array branch.

    Upstream pywayland never assigns args_ptr[i].a (and ffi.new("void []")
    raises), so Array arguments are reimplemented here; everything else
    matches upstream.
    """
    nargs = len(list(self._marshaled_arguments))
    args_ptr = ffi.new("union wl_argument []", nargs)

    arg_iter = iter(args)
    refs = []
    for i, argument in enumerate(self._marshaled_arguments):
        if argument.argument_type == ArgumentType.NewId:
            args_ptr[i].o = ffi.NULL
            continue

        arg = next(arg_iter)
        if argument.argument_type == ArgumentType.Int:
            args_ptr[i].i = arg
        elif argument.argument_type == ArgumentType.Uint:
            args_ptr[i].u = arg
        elif argument.argument_type == ArgumentType.Fixed:
            if isinstance(arg, int):
                f = lib.wl_fixed_from_int(arg)
            else:
                f = lib.wl_fixed_from_double(arg)
            args_ptr[i].f = f
        elif argument.argument_type == ArgumentType.FileDescriptor:
            args_ptr[i].h = arg
        elif argument.argument_type == ArgumentType.String:
            if arg is None:
                new_arg = ffi.NULL
            else:
                new_arg = ffi.new("char []", arg.encode())
                refs.append(new_arg)
            args_ptr[i].s = new_arg
        elif argument.argument_type == ArgumentType.Object:
            if arg is None:
                new_arg = ffi.NULL
            else:
                new_arg = ffi.cast("struct wl_object *", arg._ptr)
                refs.append(new_arg)
            args_ptr[i].o = new_arg
        elif argument.argument_type == ArgumentType.Array:
            data = bytes(arg)
            new_arg = ffi.new("struct wl_array *")
            if data:
                new_data = ffi.new("char []", data)
                new_arg.data = new_data
            else:
                new_data = ffi.NULL
                new_arg.data = ffi.NULL
            new_arg.alloc = new_arg.size = len(data)
            args_ptr[i].a = new_arg
            refs.append(new_arg)
            refs.append(new_data)

    if len(refs) > 0:
        _array_refs[int(ffi.cast("uintptr_t", args_ptr))] = refs

    return args_ptr


_array_refs: dict[int, list] = {}
Message.arguments_to_c = _fixed_arguments_to_c


def _noop_handlers(resource, exclude=()):
    """Assign no-op handlers for every request of a resource."""
    for name in resource.dispatcher._names:
        if name not in exclude:
            resource.dispatcher[name] = lambda *a, **kw: None


# ---------------------------------------------------------------------------
# Compositor state
# ---------------------------------------------------------------------------

MINIMAL_KEYMAP = b"""xkb_keymap {
xkb_keycodes "empty" {
    minimum = 8;
    maximum = 255;
};
xkb_types "empty" {
    include "basic"
};
xkb_compat "empty" {
    include "complete"
};
xkb_symbols "empty" {
    name[Group1] = "Empty";
};
};
"""


class Compositor:
    def __init__(self, socket_name: str):
        self.display = Display()
        self.display.add_socket(socket_name)
        self.display.init_shm()  # libwayland handles wl_shm internally
        self.loop = self.display.get_event_loop()

        self.surfaces: dict[int, dict] = {}  # wl_surface ptr key -> state
        self.keyboards: list = []  # WlKeyboard resources (all clients)
        self.focused_surface = None
        self.vkbd_keymap: bytes | None = None
        self.pressed_keys: list[int] = []
        self._wm_bases: list = []  # XdgWmBase resources for pinging
        self._last_ping = 0.0

        self._create_globals()

    # -- globals ----------------------------------------------------------

    def _create_globals(self):
        # Keep references: the Global's ffi handle must stay alive for binds
        self._globals = []
        comp = WlCompositor.global_class(self.display, 4)
        comp.bind_func = self._on_compositor_bind

        seat = WlSeat.global_class(self.display, 7)
        seat.bind_func = self._on_seat_bind

        output = WlOutput.global_class(self.display, 4)
        output.bind_func = self._on_output_bind

        wm = XdgWmBase.global_class(self.display, 6)
        wm.bind_func = self._on_wm_base_bind

        vkbd_mgr = ZwpVirtualKeyboardManagerV1.global_class(self.display, 1)
        vkbd_mgr.bind_func = self._on_vkbd_manager_bind

        self._globals += [comp, seat, output, wm, vkbd_mgr]

    # -- wl_compositor -----------------------------------------------------

    def _on_compositor_bind(self, resource):
        _fix_dispatcher(resource)

        def create_surface(res, surface):
            _noop_handlers(surface, exclude=("attach", "commit", "frame", "destroy"))
            state = {"resource": surface, "xdg": None, "toplevel": None}
            self.surfaces[int(ffi.cast("uintptr_t", surface._ptr))] = state

            def attach(r, buffer, x, y):
                # We never read buffer contents, so release it immediately —
                # Qt's SHM backing store blocks waiting for release events
                # before reusing buffers.
                if buffer is not None:
                    buffer.dispatcher["destroy"] = lambda res: res.destroy()
                    buffer.release()

            def commit(r):
                self._on_commit(state)

            def frame(r, callback):
                callback.done(int(time.monotonic() * 1000) & 0xFFFFFFFF)
                callback.destroy()

            def destroy(r):
                self.surfaces.pop(int(ffi.cast("uintptr_t", r._ptr)), None)
                if self.focused_surface is r:
                    self.focused_surface = None

            surface.dispatcher["attach"] = attach
            surface.dispatcher["commit"] = commit
            surface.dispatcher["frame"] = frame
            surface.dispatcher["destroy"] = destroy

        resource.dispatcher["create_surface"] = create_surface
        resource.dispatcher["create_region"] = lambda res, region: _noop_handlers(region)

    # -- wl_seat ------------------------------------------------------------

    def _on_seat_bind(self, resource):
        _fix_dispatcher(resource)
        resource.capabilities(WlSeat.capability.keyboard)
        if resource.version >= 2:
            resource.name("dictation-test-seat")

        def get_keyboard(res, keyboard):
            _dbg("get_keyboard")
            _noop_handlers(keyboard, exclude=("release",))
            self.keyboards.append(keyboard)
            fd = os.memfd_create("initial-keymap")
            os.write(fd, MINIMAL_KEYMAP)
            os.lseek(fd, 0, os.SEEK_SET)
            # NB: no close(fd) — libwayland owns queued fds after posting
            keyboard.keymap(1, fd, len(MINIMAL_KEYMAP))
            if keyboard.version >= 4:
                keyboard.repeat_info(25, 600)
            # If a surface is already focused and belongs to the same client,
            # send enter so late-bound keyboards also get input.
            if self.focused_surface is not None:
                self._send_enter(keyboard)
            keyboard.dispatcher["release"] = lambda r: (
                self.keyboards.remove(r) if r in self.keyboards else None
            )

        resource.dispatcher["get_keyboard"] = get_keyboard
        resource.dispatcher["get_pointer"] = lambda res, ptr: _noop_handlers(ptr)
        resource.dispatcher["get_touch"] = lambda res, touch: _noop_handlers(touch)
        resource.dispatcher["release"] = lambda r: None

    # -- wl_output -----------------------------------------------------------

    def _on_output_bind(self, resource):
        _fix_dispatcher(resource)
        resource.geometry(0, 0, 265, 165, 0, "dictation", "headless", 0)
        resource.mode(3, 1920, 1080, 60000)  # current | preferred
        if resource.version >= 2:
            resource.scale(1)
        if resource.version >= 4:
            resource.name("HEADLESS-1")
            resource.description("dictation test output")
        resource.done()
        resource.dispatcher["release"] = lambda r: None

    # -- xdg_wm_base ---------------------------------------------------------

    def _on_wm_base_bind(self, resource):
        _fix_dispatcher(resource)
        self._wm_bases.append(resource)

        def get_xdg_surface(res, xdg_surface, wl_surface):
            _noop_handlers(xdg_surface, exclude=("get_toplevel", "destroy"))
            key = int(ffi.cast("uintptr_t", wl_surface._ptr))
            state = self.surfaces.get(key)
            if state is not None:
                state["xdg"] = xdg_surface

            def get_toplevel(r, toplevel):
                _noop_handlers(toplevel)
                if state is not None:
                    state["toplevel"] = toplevel
                # states: [4] = XDG_TOPLEVEL_STATE_ACTIVATED — Qt only
                # delivers key events to an activated window
                toplevel.configure(0, 0, (4).to_bytes(4, "little"))
                r.configure(self.display.next_serial())

            xdg_surface.dispatcher["get_toplevel"] = get_toplevel
            xdg_surface.dispatcher["get_popup"] = lambda r, popup, parent, pos: None

        resource.dispatcher["get_xdg_surface"] = get_xdg_surface
        resource.dispatcher["create_positioner"] = lambda res, pos: _noop_handlers(pos)
        resource.dispatcher["pong"] = lambda res, serial: None
        resource.dispatcher["destroy"] = lambda res: None

    # -- focus / keyboard forwarding -----------------------------------------

    def _on_commit(self, state):
        _dbg(f"commit toplevel={state['toplevel'] is not None} focused={self.focused_surface is not None}")
        if state["toplevel"] is not None and self.focused_surface is None:
            self.focused_surface = state["resource"]
            for keyboard in self.keyboards:
                self._send_enter(keyboard)

    def _send_enter(self, keyboard):
        if self.focused_surface is None:
            return
        # Never reference another client's surface: the object id would be
        # unknown on this keyboard's connection (fatal protocol error).
        if lib.wl_resource_get_client(keyboard._ptr) != lib.wl_resource_get_client(
            self.focused_surface._ptr
        ):
            return
        _dbg("send enter")
        keyboard.enter(
            self.display.next_serial(),
            self.focused_surface,
            b"".join(
                k.to_bytes(4, "little") for k in self.pressed_keys
            ),
        )
        # Replay the current keymap so the client has the right one
        if self.vkbd_keymap is not None:
            self._send_keymap(keyboard, self.vkbd_keymap)

    def _send_keymap(self, keyboard, keymap: bytes):
        fd = os.memfd_create("vkbd-keymap")
        os.write(fd, keymap)
        os.lseek(fd, 0, os.SEEK_SET)
        # NB: no close(fd) — libwayland owns queued fds after posting
        keyboard.keymap(1, fd, len(keymap))

    def _focused_keyboards(self):
        """Keyboards belonging to the client that owns the focused surface."""
        if self.focused_surface is None:
            return []
        client = lib.wl_resource_get_client(self.focused_surface._ptr)
        return [
            k for k in self.keyboards
            if lib.wl_resource_get_client(k._ptr) == client
        ]

    # -- zwp_virtual_keyboard_manager_v1 --------------------------------------

    def _on_vkbd_manager_bind(self, resource):
        _fix_dispatcher(resource)

        def create_virtual_keyboard(res, seat, vkbd):
            def keymap(r, fmt, fd, size):
                _dbg(f"vkbd keymap size={size}")
                self.vkbd_keymap = os.read(fd, size)
                os.close(fd)
                for keyboard in self._focused_keyboards():
                    self._send_keymap(keyboard, self.vkbd_keymap)

            def key(r, time_ms, key_code, state):
                _dbg(f"vkbd key code={key_code} state={state} keyboards={len(self.keyboards)} focused={self.focused_surface is not None}")
                if state == 1 and key_code not in self.pressed_keys:
                    self.pressed_keys.append(key_code)
                elif state == 0 and key_code in self.pressed_keys:
                    self.pressed_keys.remove(key_code)
                serial = self.display.next_serial()
                for keyboard in self._focused_keyboards():
                    keyboard.key(serial, time_ms, key_code, state)
                self.display.flush_clients()
                _dbg("flushed")

            def modifiers(r, depressed, latched, locked, group):
                serial = self.display.next_serial()
                for keyboard in self._focused_keyboards():
                    keyboard.modifiers(serial, depressed, latched, locked, group)

            vkbd.dispatcher["keymap"] = keymap
            vkbd.dispatcher["key"] = key
            vkbd.dispatcher["modifiers"] = modifiers
            vkbd.dispatcher["destroy"] = lambda r: None

        resource.dispatcher["create_virtual_keyboard"] = create_virtual_keyboard

    # -- main loop -------------------------------------------------------------

    def run(self):
        print("READY", flush=True)
        try:
            while True:
                self.loop.dispatch(500)
                # Periodically ping clients (connection-liveness probe)
                if DEBUG and time.monotonic() - self._last_ping > 0.5:
                    self._last_ping = time.monotonic()
                    for wm in self._wm_bases:
                        wm.ping(self.display.next_serial())
                self.display.flush_clients()
        except KeyboardInterrupt:
            pass


def main():
    socket_name = sys.argv[1] if len(sys.argv) > 1 else "dictation-test"
    compositor = Compositor(socket_name)
    compositor.run()


if __name__ == "__main__":
    main()
