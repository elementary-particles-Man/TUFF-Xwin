#!/usr/bin/env python3
"""Two real Wayland protocol clients used by the live topology smoke test."""
import array
import os
import socket
import struct
import sys
import time


def raw(obj, opcode, payload=b""):
    return struct.pack("<II", obj, ((8 + len(payload)) << 16) | opcode) + payload


def words(obj, opcode, *values):
    return raw(obj, opcode, b"".join(struct.pack("<I", v & 0xFFFFFFFF) for v in values))


def string_arg(value):
    data = value.encode() + b"\0"
    data += b"\0" * ((4 - len(data) % 4) % 4)
    return struct.pack("<I", len(value) + 1) + data


class Client:
    def __init__(self, path, tag):
        self.tag = tag
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(0.25)
        self.sock.connect(path)
        self.buf = bytearray()
        self.globals = {}
        self.next_id = 20 if tag == "A" else 40
        self.callback = None
        self.outputs_seen = set()
        self._bind_registry()
        self._create_surface()

    def alloc(self):
        value = self.next_id
        self.next_id += 1
        return value

    def send(self, data, fd=None):
        obj, header = struct.unpack_from("<II", data)
        size, opcode = header >> 16, header & 0xFFFF
        if size != len(data):
            raise RuntimeError(f"{self.tag}: invalid request size {size}/{len(data)}")
        print(f"client={self.tag} request object={obj} opcode={opcode} size={size} payload={len(data)-8} fds={1 if fd is not None else 0}", flush=True)
        if fd is None:
            self.sock.sendall(data)
            return
        fds = array.array("i", [fd])
        sent = self.sock.sendmsg([data], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, fds)])
        if sent != len(data):
            raise RuntimeError(f"{self.tag}: short fd send {sent}/{len(data)}")

    def receive(self, timeout):
        deadline = time.monotonic() + timeout
        while True:
            while len(self.buf) >= 8:
                obj, header = struct.unpack_from("<II", self.buf)
                size, opcode = header >> 16, header & 0xFFFF
                if size < 8 or len(self.buf) < size:
                    break
                payload = bytes(self.buf[8:size])
                del self.buf[:size]
                yield obj, opcode, payload
            left = deadline - time.monotonic()
            if left <= 0:
                return
            self.sock.settimeout(left)
            try:
                chunk = self.sock.recv(65536)
            except socket.timeout:
                return
            if not chunk:
                raise RuntimeError(f"{self.tag}: unexpected EOF")
            self.buf.extend(chunk)

    def wait_for(self, predicate, timeout=4.0):
        for event in self.receive(timeout):
            if event[0] in self._output_objects and event[1] in (0, 1, 2, 3, 4):
                self.outputs_seen.add(event[0])
            if predicate(*event):
                return event
        raise RuntimeError(f"{self.tag}: protocol response timeout")

    def _bind_registry(self):
        self.registry = 2
        self.send(words(1, 1, self.registry))
        self.send(words(1, 0, 3))
        self._output_objects = set()
        for event in self.receive(3.0):
            obj, opcode, payload = event
            if obj == self.registry and opcode == 0 and len(payload) >= 8:
                name, length = struct.unpack_from("<II", payload)
                end = 8 + length
                interface = payload[8:end].split(b"\0", 1)[0].decode()
                version = struct.unpack_from("<I", payload, (end + 3) & ~3)[0]
                self.globals[interface] = (name, version)
                if interface == "wl_output":
                    self.outputs_seen.add(name)
            if obj == 3 and opcode == 0:
                break
        required = {"wl_compositor", "wl_shm", "xdg_wm_base"}
        if not required.issubset(self.globals):
            raise RuntimeError(f"{self.tag}: missing globals {required - self.globals.keys()}")
        self._bind("wl_compositor", 4, 4 if self.tag == "A" else 20)
        self._bind("wl_shm", 1, 7 if self.tag == "A" else 23)
        self._bind("xdg_wm_base", 6, 10 if self.tag == "A" else 26)

    def _bind(self, interface, version, new_id):
        name, advertised = self.globals[interface]
        payload = struct.pack("<I", name) + string_arg(interface)
        payload += struct.pack("<II", min(version, advertised), new_id)
        self.send(raw(self.registry, 0, payload))

    def _create_surface(self):
        base = 4 if self.tag == "A" else 20
        self.compositor, self.surface, self.shm = base, base + 1, base + 3
        self.pool, self.buffer = base + 4, base + 7
        self.xdg, self.xdg_surface, self.toplevel = base + 6, base + 2, base + 5
        self.send(words(self.compositor, 0, self.surface))
        self.temp = open(os.path.join("/tmp", f"wayland-live-{os.getpid()}-{self.tag}"), "w+b")
        self.temp.truncate(0)
        self.send(raw(self.shm, 0, struct.pack("<II", self.pool, 4096)), self.temp.fileno())
        self.send(raw(self.pool, 0, struct.pack("<IIIIII", self.buffer, 0, 1, 1, 4, 0)))
        self.send(raw(self.xdg, 2, struct.pack("<II", self.xdg_surface, self.surface)))
        self.send(raw(self.xdg_surface, 1, struct.pack("<I", self.toplevel)))
        serial = self.wait_for(lambda obj, opcode, payload: obj == self.xdg_surface and opcode == 0)[2][:4]
        self.send(raw(self.xdg_surface, 3, struct.pack("<iiii", 0, 0, 1, 1)))
        self.send(raw(self.xdg_surface, 4, serial))

    def commit(self):
        self.callback = self.alloc()
        self.send(words(self.surface, 1, self.buffer, 0, 0))
        self.send(raw(self.surface, 2, struct.pack("<iiii", 0, 0, 1, 1)))
        self.send(words(self.surface, 3, self.callback))
        self.send(raw(self.surface, 6))
        print(f"client={self.tag} surface_commit callback={self.callback}", flush=True)

    def sync(self):
        callback = self.alloc()
        self.send(words(1, 0, callback))
        self.wait_for(lambda obj, opcode, payload: obj == callback and opcode == 0)

    def wait_callback(self, timeout=10):
        callback = self.callback
        if callback is None:
            raise RuntimeError(f"{self.tag}: callback not registered")
        event = self.wait_for(lambda obj, opcode, payload: obj == callback and opcode == 0, timeout)
        if len(event[2]) != 4:
            raise RuntimeError(f"{self.tag}: invalid callback payload")
        print(f"client={self.tag} callback_done object={callback} opcode=0", flush=True)
        self.callback = None

    def close(self):
        self.sock.close()
        self.temp.close()


def main():
    path, ready, stop, stopped, resume, done = sys.argv[1:]
    clients = [Client(path, "A"), Client(path, "B")]
    for client in clients:
        client.commit()
    for client in clients:
        client.wait_callback(10)
    open(ready, "w").close()
    while not os.path.exists(stop):
        time.sleep(0.02)
    for client in clients:
        client.sync()
    open(stopped, "w").close()
    while not os.path.exists(resume):
        time.sleep(0.02)
    for client in clients:
        client.sync()
        client.commit()
    for client in clients:
        client.wait_callback(12)
    open(done, "w").close()
    for client in clients:
        client.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"live topology client failure: {error}", file=sys.stderr)
        sys.exit(1)
