import json
import sys
import threading
from typing import Any, Callable, Optional


class Host:
    def __init__(self, plugin: "Plugin"):
        self._plugin = plugin

    @property
    def config(self) -> dict[str, Any]:
        return self._plugin.config

    @property
    def initialize_params(self) -> dict[str, Any]:
        return self._plugin.initialize_params

    def request(self, method: str, params: Optional[dict[str, Any]] = None, timeout: float = 30.0):
        return self._plugin.request_host(method, params or {}, timeout)

    def notify(self, method: str, params: Optional[dict[str, Any]] = None):
        self._plugin.write({"jsonrpc": "2.0", "method": method, "params": params or {}})

    def log(self, level: str, message: str):
        self.notify("log", {"level": level, "message": message})

    def asset_read(self, path: str):
        return self.request("assets.read", {"path": path})

    def asset_write(self, path: str, content: str, append: bool = False):
        return self.request("assets.write", {"path": path, "content": content, "append": append})

    def asset_list(self, path: str = ""):
        return self.request("assets.list", {"path": path})

    def cache_read(self, path: str):
        return self.request("cache.read", {"path": path})

    def cache_write(self, path: str, content: str, append: bool = False):
        return self.request("cache.write", {"path": path, "content": content, "append": append})

    def cache_clear(self):
        return self.request("cache.clear", {})

    def secret_set(self, key: str, value: str):
        return self.request("secrets.set", {"key": key, "content": value})

    def secret_get(self, key: str):
        return self.request("secrets.get", {"key": key})


class Plugin:
    def __init__(
        self,
        methods: Optional[dict[str, Callable[[dict[str, Any], Host, dict[str, Any]], Any]]] = None,
        initialize: Optional[Callable[[dict[str, Any], Host, dict[str, Any]], Any]] = None,
        notifications: Optional[dict[str, Callable[[dict[str, Any], Host, dict[str, Any]], Any]]] = None,
    ):
        self.methods = methods or {}
        self.initialize = initialize
        self.notifications = notifications or {}
        self.config: dict[str, Any] = {}
        self.initialize_params: dict[str, Any] = {}
        self._next_host_request_id = 10000
        self._pending: dict[int, tuple[threading.Event, dict[str, Any]]] = {}
        self.host = Host(self)

    def start(self):
        for line in sys.stdin:
            try:
                message = json.loads(line.strip())
            except Exception:
                continue
            if not message or message.get("jsonrpc") != "2.0":
                continue
            if "id" in message and "method" not in message:
                self.resolve_host_response(message)
            elif "id" in message and "method" in message:
                self.handle_request(message)
            elif "method" in message:
                self.handle_notification(message)

    def handle_request(self, req: dict[str, Any]):
        req_id = req.get("id")
        method = req.get("method")
        params = req.get("params") or {}
        try:
            if method == "initialize":
                self.initialize_params = params
                self.config = params.get("config") or {}
                result = (
                    self.initialize(params, self.host, req)
                    if self.initialize
                    else {"ok": True}
                )
                self.respond(req_id, result or {"ok": True})
                return
            handler = self.methods.get(method)
            if not handler:
                self.respond_error(req_id, -32601, f"Method not found: {method}")
                return
            result = handler(params, self.host, req)
            self.respond(req_id, result if result is not None else {"ok": True})
        except Exception as exc:
            self.respond_error(req_id, -32603, str(exc))

    def handle_notification(self, notif: dict[str, Any]):
        method = notif.get("method")
        params = notif.get("params") or {}
        if method == "config.update":
            self.config = params.get("config") or {}
        if method == "shutdown" and method not in self.notifications:
            sys.exit(0)
        handler = self.notifications.get(method)
        if handler:
            try:
                handler(params, self.host, notif)
            except Exception as exc:
                self.host.log("error", str(exc))

    def request_host(self, method: str, params: dict[str, Any], timeout: float):
        request_id = self._next_host_request_id
        self._next_host_request_id += 1
        event = threading.Event()
        box: dict[str, Any] = {}
        self._pending[request_id] = (event, box)
        self.write({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        if not event.wait(timeout):
            self._pending.pop(request_id, None)
            raise TimeoutError(f"Host request timeout: {method}")
        if "error" in box:
            raise RuntimeError(box["error"])
        return box.get("result") or {}

    def resolve_host_response(self, message: dict[str, Any]):
        request_id = message.get("id")
        pending = self._pending.pop(request_id, None)
        if not pending:
            return
        event, box = pending
        if message.get("error"):
            box["error"] = (message.get("error") or {}).get("message") or "Host request failed"
        else:
            box["result"] = message.get("result") or {}
        event.set()

    def respond(self, req_id: int, result: Any):
        self.write({"jsonrpc": "2.0", "id": req_id, "result": result})

    def respond_error(self, req_id: int, code: int, message: str):
        self.write({"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}})

    def write(self, message: dict[str, Any]):
        sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
        sys.stdout.flush()


def create_plugin(**kwargs) -> Plugin:
    return Plugin(**kwargs)
