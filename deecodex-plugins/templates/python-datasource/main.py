import time

from deecodex_plugin import create_plugin

config = {}


def initialize(params, _host, _req):
    global config
    config = params.get("config") or {}
    return {"ok": True, "name": "Example Python Datasource"}


def config_update(params, _host, _notif):
    global config
    config = params.get("config") or {}


def datasource_status(_params, _host, _req):
    return {"ok": True, "root": config.get("root", "."), "ts": int(time.time())}


def datasource_search(params, _host, _req):
    query = params.get("query", "")
    return {"query": query, "items": []}


def datasource_read(_params, _host, _req):
    return {"content": ""}


create_plugin(
    initialize=initialize,
    notifications={"config.update": config_update},
    methods={
        "datasource.status": datasource_status,
        "datasource.search": datasource_search,
        "datasource.read": datasource_read,
    },
).start()
