#!/usr/bin/env python3
"""開發用:透過 API 建立示範店家 demo-salon(2 位美髮師、3 項服務、每天營業)。

只用標準函式庫,只打 HTTP API,不碰資料庫。可重複執行(已存在就跳過)。
    python3 scripts/seed_demo.py [http://127.0.0.1:3001]
"""
import json
import sys
import urllib.error
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:3001"
SLUG = "demo-salon"
PASSWORD = "demo-password-123"
OWNER = "owner@demo.example.com"
STYLIST = "stylist@demo.example.com"


def call(method, path, body=None, token=None):
    req = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        return e.code, (json.loads(raw) if raw else None)


def login_or_register(email, name):
    status, body = call("POST", "/auth/register", {"email": email, "password": PASSWORD, "name": name})
    if status == 201:
        return body["token"], body["user"]["id"]
    status, body = call("POST", "/auth/login", {"email": email, "password": PASSWORD})
    assert status == 200, (status, body)
    return body["token"], body["user"]["id"]


owner_token, owner_id = login_or_register(OWNER, "林美玲")
stylist_token, stylist_id = login_or_register(STYLIST, "陳小安")

status, body = call("POST", "/tenants", {"slug": SLUG, "name": "森林系髮廊", "timezone": "Asia/Taipei"}, owner_token)
if status == 201:
    print("已建立店家", SLUG)
    status, inv = call("POST", f"/t/{SLUG}/invitations", {"email": STYLIST, "role": "staff"}, owner_token)
    assert status == 201, (status, inv)
    status, _ = call("POST", "/invitations/accept", {"token": inv["token"]}, stylist_token)
    assert status == 200
    services = []
    for name, minutes, price in [("洗髮護理", 30, 0), ("剪髮", 60, 50000), ("染髮", 120, 180000)]:
        status, svc = call("POST", f"/t/{SLUG}/services", {"name": name, "duration_minutes": minutes, "price_cents": price}, owner_token)
        assert status == 201, (status, svc)
        services.append(svc["id"])
    hours = [{"weekday": d, "start": "10:00", "end": "19:00"} for d in range(1, 7)]  # 週一到週六
    for uid in (owner_id, stylist_id):
        assert call("PUT", f"/t/{SLUG}/members/{uid}/working-hours", {"hours": hours}, owner_token)[0] == 200
    assert call("PUT", f"/t/{SLUG}/members/{owner_id}/services", {"service_ids": services}, owner_token)[0] == 200
    # 第二位只做剪髮與洗髮,用來看「不同服務可選的人不同」
    assert call("PUT", f"/t/{SLUG}/members/{stylist_id}/services", {"service_ids": services[:2]}, owner_token)[0] == 200
else:
    print("店家已存在,略過:", status, body and body.get("error"))

print(f"\n預約頁:/s/{SLUG}\n員工登入:{OWNER} / {PASSWORD}")
