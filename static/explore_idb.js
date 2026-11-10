// deecodex IndexedDB 探查脚本 — 枚举 Codex 的所有数据库、表结构、样本数据
// 通过 CDP 注入 Codex 渲染进程，结果通过桥接通道返回 Rust 侧记录

(function () {
    "use strict";

    async function explore() {
        var report = {};

        // 1. 枚举所有 IndexedDB 数据库
        var dbInfos = [];
        try {
            dbInfos = await indexedDB.databases();
        } catch (e) {
            report.error = "indexedDB.databases() 失败: " + String(e);
            return report;
        }

        report.databaseCount = dbInfos.length;
        report.databases = [];

        for (var i = 0; i < dbInfos.length; i++) {
            var dbInfo = dbInfos[i];
            var dbName = dbInfo.name;
            var dbVersion = dbInfo.version;

            var dbEntry = {
                name: dbName,
                version: dbVersion,
                objectStores: []
            };

            // 2. 打开数据库，读取 schema
            try {
                var schema = await openAndReadSchema(dbName, dbVersion);
                dbEntry.objectStores = schema;
            } catch (e) {
                dbEntry.error = String(e);
            }

            report.databases.push(dbEntry);
        }

        return report;
    }

    function openAndReadSchema(dbName, version) {
        return new Promise(function (resolve, reject) {
            var request = indexedDB.open(dbName, version);
            var stores = [];

            request.onsuccess = function (event) {
                var db = event.target.result;
                var storeNames = Array.from(db.objectStoreNames);

                // 对每个 object store 读取 schema 和样本数据
                var pending = storeNames.length;
                if (pending === 0) {
                    db.close();
                    resolve(stores);
                    return;
                }

                storeNames.forEach(function (storeName) {
                    readStoreInfo(db, storeName).then(function (storeInfo) {
                        stores.push(storeInfo);
                        pending--;
                        if (pending === 0) {
                            db.close();
                            resolve(stores);
                        }
                    }).catch(function (e) {
                        stores.push({ name: storeName, error: String(e) });
                        pending--;
                        if (pending === 0) {
                            db.close();
                            resolve(stores);
                        }
                    });
                });
            };

            request.onerror = function () {
                reject(new Error("无法打开数据库: " + dbName));
            };

            // 超时 10 秒
            setTimeout(function () {
                reject(new Error("打开数据库超时: " + dbName));
            }, 10000);
        });
    }

    function readStoreInfo(db, storeName) {
        return new Promise(function (resolve) {
            try {
                var transaction = db.transaction(storeName, "readonly");
                var store = transaction.objectStore(storeName);

                var storeInfo = {
                    name: storeName,
                    keyPath: store.keyPath || null,
                    autoIncrement: store.autoIncrement,
                    indexNames: Array.from(store.indexNames),
                    sampleCount: 0,
                    samples: []
                };

                // 读计数
                var countRequest = store.count();
                countRequest.onsuccess = function () {
                    storeInfo.sampleCount = countRequest.result;
                };

                // 读取前 5 条样本数据
                var cursorRequest = store.openCursor();
                var sampleLimit = 5;
                cursorRequest.onsuccess = function (event) {
                    var cursor = event.target.result;
                    if (cursor && storeInfo.samples.length < sampleLimit) {
                        storeInfo.samples.push({
                            key: safeStringify(cursor.key),
                            value: safeStringify(cursor.value)
                        });
                        cursor.continue();
                    } else {
                        resolve(storeInfo);
                    }
                };
                cursorRequest.onerror = function () {
                    resolve(storeInfo);
                };
            } catch (e) {
                resolve({ name: storeName, error: String(e) });
            }
        });
    }

    function safeStringify(value) {
        try {
            var json = JSON.stringify(value, function (key, val) {
                // 截断过长的字符串
                if (typeof val === "string" && val.length > 500) {
                    return val.substring(0, 500) + "...[truncated]";
                }
                return val;
            }, 2);
            // 限制整体大小
            if (json.length > 2000) {
                return json.substring(0, 2000) + "...[truncated]";
            }
            return json;
        } catch (e) {
            return "[unserializable] " + String(e);
        }
    }

    // ── 执行探查，通过桥接返回结果 ──
    explore().then(function (report) {
        if (window.__deecodexBridge) {
            window.__deecodexBridge("/idb-report", report).then(function (result) {
                console.log("[deecodex] IndexedDB 探查完成:", result);
            });
        } else {
            // 回退：输出到 console（可通过 CDP Runtime.consoleAPICalled 捕获）
            console.log("[deecodex] IndexedDB 探查报告 (console 回退):", JSON.stringify(report, null, 2));
        }
    }).catch(function (e) {
        console.error("[deecodex] IndexedDB 探查失败:", e);
    });
})();
