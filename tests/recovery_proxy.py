"""A separate-process TCP relay so faults also work while Rust holds the GIL."""

from multiprocessing import get_context
from multiprocessing.connection import wait
import select
import selectors
import socket
import time


def _serve(control):  # noqa: C901, PLR0912, PLR0915
    selector = selectors.DefaultSelector()
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    selector.register(listener, selectors.EVENT_READ)
    selector.register(control, selectors.EVENT_READ)
    peers = {}
    accepted = 0
    timestamps = []
    held = []
    allowed = True
    budget = None
    paused = False

    def disconnect():
        for stream in held:
            stream.close()
        held.clear()
        for stream in list(peers):
            if stream in selector.get_map():
                selector.unregister(stream)
            stream.close()
        peers.clear()

    def attach(client):
        upstream = socket.create_connection(("127.0.0.1", 8022), timeout=5)
        upstream.settimeout(None)
        peers[client] = upstream
        peers[upstream] = client
        if not paused:
            selector.register(client, selectors.EVENT_READ)
            selector.register(upstream, selectors.EVENT_READ)

    control.send(listener.getsockname()[1])
    try:
        running = True
        while running:
            for event, _ in selector.select():
                stream = event.fileobj
                if stream is control:
                    command, value = control.recv()
                    if command == "drop":
                        disconnect()
                        allowed = value
                    elif command == "allow":
                        allowed = value
                        if allowed is True:
                            for client in held:
                                attach(client)
                            held.clear()
                    elif command == "pause":
                        if paused != value:
                            paused = value
                            for peer in peers:
                                if paused:
                                    selector.unregister(peer)
                                else:
                                    selector.register(peer, selectors.EVENT_READ)
                    elif command == "pending":
                        control.send(len(select.select(list(peers), [], [], 0)[0]))
                        continue
                    elif command == "times":
                        control.send(timestamps)
                        continue
                    elif command == "arm":
                        budget = value
                    elif command == "stop":
                        running = False
                    control.send(accepted)
                elif stream is listener:
                    client, _ = listener.accept()
                    accepted += 1
                    timestamps.append(time.monotonic())
                    if allowed == "hold":
                        held.append(client)
                        continue
                    if not allowed:
                        client.close()
                        continue
                    attach(client)
                elif stream in peers and not paused:
                    try:
                        data = stream.recv(65536)
                    except OSError:
                        disconnect()
                        continue
                    if budget is not None:
                        budget -= len(data)
                        if budget <= 0:
                            budget = None
                            disconnect()
                            continue
                    if data:
                        try:
                            peers[stream].sendall(data)
                        except OSError:
                            disconnect()
                    else:
                        other = peers.pop(stream)
                        peers.pop(other)
                        for closed in (stream, other):
                            selector.unregister(closed)
                            closed.close()
    finally:
        disconnect()
        listener.close()
        selector.close()
        control.close()


class FaultProxy:
    """Acknowledge faults only after closing the active TCP sockets."""

    def __init__(self):
        context = get_context("spawn")
        self.control, child = context.Pipe()
        self.process = context.Process(target=_serve, args=(child,), daemon=True)
        self.process.start()
        child.close()
        assert wait([self.control], timeout=10), "TCP relay did not start"
        self.port = self.control.recv()

    def command(self, name, value=None):
        self.control.send((name, value))
        assert wait([self.control], timeout=10), "TCP relay did not acknowledge fault"
        return self.control.recv()

    def drop(self, *, allow=True):
        return self.command("drop", allow)

    @property
    def attempts(self):
        return self.command("count")

    def close(self):
        if self.process.is_alive():
            self.command("stop")
            self.process.join(timeout=10)
        if self.process.is_alive():
            self.process.terminate()
            self.process.join(timeout=10)
        self.control.close()
