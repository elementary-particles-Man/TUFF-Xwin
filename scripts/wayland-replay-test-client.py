#!/usr/bin/env python3
"""Two independent, standard-library-only Wayland protocol clients for replay smoke tests."""
import array
import os
import socket
import struct
import sys
import time


def msg(obj, opcode, *words):
    payload = b"".join(struct.pack("<I", word & 0xFFFFFFFF) for word in words)
    return struct.pack("<II", obj, (8 + len(payload)) << 16 | opcode) + payload


def raw_msg(obj, opcode, payload=b""):
    return struct.pack("<II", obj, (8 + len(payload)) << 16 | opcode) + payload


def string_arg(value):
    raw = value.encode() + b"\0"
    raw += b"\0" * ((4 - len(raw) % 4) % 4)
    return struct.pack("<I", len(value) + 1) + raw


class Client:
    def __init__(self, path, tag):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(0.25)
        self.sock.connect(path)
        self.tag = tag
        self.buf = bytearray()
        self.globals = {}
        self.next_id = 20 if tag == "A" else 40
        self.callback = None
        self._registry_roundtrip()
        self._bind_and_surface()

    def alloc(self):
        value = self.next_id
        self.next_id += 1
        return value

    def send(self, data, fd=None):
        obj, word = struct.unpack_from("<II", data)
        declared_size, opcode = word >> 16, word & 0xFFFF
        payload_len = len(data) - 8
        if declared_size != len(data):
            raise RuntimeError(
                f"client {self.tag} malformed outbound object={obj} opcode={opcode} "
                f"size={declared_size} payload={payload_len} bytes={len(data)}"
            )
        print(
            f"client={self.tag} request object={obj} opcode={opcode} size={declared_size} "
            f"payload={payload_len} fds={1 if fd is not None else 0}",
            flush=True,
        )
        if fd is None:
            self.sock.sendall(data)
        else:
            fds = array.array("i", [fd])
            sent = self.sock.sendmsg([data], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, fds)])
            if sent != len(data):
                raise RuntimeError(f"client {self.tag} short SCM_RIGHTS send {sent}/{len(data)}")

    def receive(self, timeout=2.0):
        deadline = time.monotonic() + timeout
        while True:
            while len(self.buf) >= 8:
                obj, word = struct.unpack_from("<II", self.buf)
                size, opcode = word >> 16, word & 0xFFFF
                if size < 8 or len(self.buf) < size:
                    break
                payload = bytes(self.buf[8:size])
                del self.buf[:size]
                yield obj, opcode, payload
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            self.sock.settimeout(remaining)
            try:
                chunk = self.sock.recv(65536)
            except socket.timeout:
                return
            if not chunk:
                raise RuntimeError(f"client {self.tag} received EOF")
            self.buf.extend(chunk)

    def wait_for(self, predicate, timeout=3.0):
        for item in self.receive(timeout):
            if predicate(*item):
                return item
        raise RuntimeError(f"client {self.tag} timed out waiting for protocol response")

    def _registry_roundtrip(self):
        print(f"client {self.tag}: registry", flush=True)
        self.send(msg(1, 1, 2))
        self.send(msg(1, 0, 3))
        def global_event(obj, opcode, payload):
            if obj != 2 or opcode != 0 or len(payload) < 8:
                return False
            name, length = struct.unpack_from("<II", payload)
            end = 8 + length
            interface = payload[8:end].split(b"\0", 1)[0].decode()
            version = struct.unpack_from("<I", payload, (end + 3) & ~3)[0]
            self.globals[interface] = (name, version)
            return False
        for item in self.receive(2.0):
            if item[0] == 3 and item[1] == 0:
                break
            global_event(*item)
        required = {"wl_compositor", "wl_shm", "xdg_wm_base"}
        if not required.issubset(self.globals):
            raise RuntimeError(f"client {self.tag} missing globals: {required - self.globals.keys()}")

    def _bind(self, interface, version, new_id):
        name = self.globals[interface][0]
        payload = struct.pack("<I", name) + string_arg(interface)
        payload += struct.pack("<II", min(version, self.globals[interface][1]), new_id)
        self.send(raw_msg(2, 0, payload))

    def _bind_and_surface(self):
        print(f"client {self.tag}: bind and surface", flush=True)
        base = 4 if self.tag == "A" else 20
        self.compositor, self.shm, self.xdg = base, base + 3, base + 6
        self.surface, self.pool, self.buffer = base + 1, base + 4, base + 7
        self.xdg_surface, self.toplevel = base + 2, base + 5
        self._bind("wl_compositor", 4, self.compositor)
        self._bind("wl_shm", 1, self.shm)
        self._bind("xdg_wm_base", 6, self.xdg)
        self.send(raw_msg(self.compositor, 0, struct.pack("<I", self.surface)))
        temp = open(os.path.join("/tmp", f"wayland-replay-{os.getpid()}-{self.tag}"), "w+b")
        # Keep a valid shm buffer object while leaving its optional pixel
        # transport empty; canonical replay then exercises metadata and
        # callback recovery without cross-client payload generations.
        temp.truncate(0)
        self.temp = temp
        self.send(raw_msg(self.shm, 0, struct.pack("<II", self.pool, 4096)), temp.fileno())
        self.send(raw_msg(self.pool, 0, struct.pack("<IIIIII", self.buffer, 0, 1, 1, 4, 0)))
        self.send(raw_msg(self.xdg, 2, struct.pack("<II", self.xdg_surface, self.surface)))
        self.send(raw_msg(self.xdg_surface, 1, struct.pack("<I", self.toplevel)))
        serial = self.wait_for(lambda obj, opcode, payload: obj == self.xdg_surface and opcode == 0)[2]
        self.send(raw_msg(self.xdg_surface, 3, struct.pack("<iiii", 0, 0, 1, 1)))
        self.send(raw_msg(self.xdg_surface, 4, serial[:4]))

    def commit(self):
        print(f"client {self.tag}: commit", flush=True)
        self.callback = self.alloc()
        self.send(raw_msg(self.surface, 1, struct.pack("<III", self.buffer, 0, 0)))
        self.send(raw_msg(self.surface, 2, struct.pack("<iiii", 0, 0, 1, 1)))
        self.send(raw_msg(self.surface, 3, struct.pack("<I", self.callback)))
        self.send(raw_msg(self.surface, 6))

    def sync(self):
        callback = self.alloc()
        self.send(msg(1, 0, callback))
        self.wait_for(lambda obj, opcode, payload: obj == callback and opcode == 0)

    def callback_done(self, timeout=0.5):
        if self.callback is None:
            return False
        try:
            self.wait_for(lambda obj, opcode, payload: obj == self.callback and opcode == 0, timeout)
            print(
                f"client={self.tag} callback_done object={self.callback} opcode=0",
                flush=True,
            )
            self.callback = None
            return True
        except RuntimeError:
            return False


def main():
    path, ready, outage, outage_ok, resume, done = sys.argv[1:]
    clients = [Client(path, "A"), Client(path, "B")]
    open(ready, "w").close()
    while not os.path.exists(outage):
        time.sleep(0.02)
    for client in clients:
        client.commit()
    if any(client.callback_done(1.5) for client in clients):
        raise RuntimeError("callback completed while compd was stopped")
    for client in clients:
        client.sync()
    open(outage_ok, "w").close()
    while not os.path.exists(resume):
        time.sleep(0.02)
    deadline = time.monotonic() + 10
    completed = [False, False]
    while sum(completed) < 2 and time.monotonic() < deadline:
        for index, client in enumerate(clients):
            if not completed[index]:
                completed[index] = client.callback_done(0.25)
    open(done, "w").close()
    for client in clients:
        client.sock.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"wayland replay client failure: {error}", file=sys.stderr)
        sys.exit(1)
