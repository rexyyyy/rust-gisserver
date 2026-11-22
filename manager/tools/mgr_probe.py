"""系统管理后端接口探针 v2：登录 → 取信息/路由 → 各模块读写 → 监控。

用法：python manager/tools/mgr_probe.py [base_url] [out_file]
默认探测 http://127.0.0.1:6181/manager-api（探针实例，已关闭验证码）
"""
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:6181/manager-api"
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = (
    sys.argv[2]
    if len(sys.argv) > 2
    else os.path.join(ROOT, "mgr_probe.txt")
)

passed = 0
failed = 0
lines = []
created = {"post": None, "dict_type": None, "dict_data": None, "notice": None, "config": None}


def req(method, path, body=None, token=None):
    url = BASE + path
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = "Bearer " + token
    r = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(r, timeout=20) as resp:
            return resp.status, json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"msg": raw[:200]}


def check(name, cond, extra=""):
    global passed, failed
    if cond:
        passed += 1
        lines.append(f"PASS  {name} {extra}")
    else:
        failed += 1
        lines.append(f"FAIL  {name} {extra}")


def ok(code, js):
    return code == 200 and js.get("code") == 200


def dump(tag, js):
    lines.append(f"      >> {tag}: {json.dumps(js, ensure_ascii=False)[:400]}")


# 1. 登录
st, js = req("POST", "/auth/login", {"username": "admin", "password": "admin123"})
check("POST /auth/login", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("login", js)
token = (js.get("data") or {}).get("access_token", "") if isinstance(js.get("data"), dict) else ""
if not token:
    lines.append("!! 未取得 token，后续用例全部跳过")
    open(OUT, "w", encoding="utf-8").write("\n".join(lines))
    raise SystemExit(1)

# 2. getInfo / getRouters
st, js = req("GET", "/system/user/getInfo", token=token)
check("GET  /system/user/getInfo", ok(st, js) and "user" in (js.get("data") or {}), f"status={st}")
if not ok(st, js):
    dump("getInfo", js)
st, js = req("GET", "/system/menu/getRouters", token=token)
routers = js.get("data") or []
check("GET  /system/menu/getRouters", ok(st, js) and len(routers) > 0, f"status={st} 顶层路由={len(routers)}")
if len(routers) == 0:
    dump("getRouters", js)

# 3. 各模块列表
for name, path in [
    ("用户列表", "/system/user/list?pageNum=1&pageSize=10"),
    ("角色列表", "/system/role/list?pageNum=1&pageSize=10"),
    ("菜单列表", "/system/menu/list"),
    ("部门列表", "/system/dept/list"),
    ("岗位列表", "/system/post/list?pageNum=1&pageSize=10"),
    ("字典类型", "/system/dict/type/list?pageNum=1&pageSize=10"),
    ("字典数据", "/system/dict/data/list?pageNum=1&pageSize=10"),
    ("参数设置", "/system/config/list?pageNum=1&pageSize=10"),
    ("通知公告", "/system/notice/list?pageNum=1&pageSize=10"),
    ("在线用户", "/monitor/online/list"),
    ("登录日志", "/monitor/logininfor/list?pageNum=1&pageSize=10"),
    ("操作日志", "/monitor/operlog/list?pageNum=1&pageSize=10"),
]:
    st, js = req("GET", path, token=token)
    rows = js.get("rows")
    total = js.get("total")
    good = ok(st, js) and (rows is not None or isinstance(js.get("data"), list))
    check(f"GET  {name}", good, f"status={st} total={total} rows={len(rows) if isinstance(rows, list) else '-'}")
    if not good:
        dump(name, js)

# 4. 树/选择器
for name, path in [
    ("部门树", "/system/user/deptTree"),
    ("菜单树", "/system/menu/treeselect"),
    ("岗位选择", "/system/post/optionselect"),
    ("角色选择", "/system/role/optionselect"),
    ("用户选择", "/system/user/optionselect"),
    ("字典类型下拉", "/system/dict/type/optionselect"),
]:
    st, js = req("GET", path, token=token)
    check(f"GET  {name}", ok(st, js), f"status={st}")
    if not ok(st, js):
        dump(name, js)

# 5. 单条查询（路径参数路由）
for name, path in [
    ("用户详情", "/system/user/1"),
    ("角色详情", "/system/role/1"),
    ("菜单详情", "/system/menu/1"),
    ("部门详情", "/system/dept/100"),
    ("岗位详情", "/system/post/1"),
    ("字典类型详情", "/system/dict/type/1"),
    ("字典数据详情", "/system/dict/data/1"),
    ("参数详情", "/system/config/1"),
    ("公告详情", "/system/notice/1"),
]:
    st, js = req("GET", path, token=token)
    check(f"GET  {name}", ok(st, js), f"status={st}")
    if not ok(st, js):
        dump(name, js)

# 6. 写操作闭环：岗位
st, js = req("POST", "/system/post", {
    "postName": "探针岗位", "postCode": "probe", "postSort": 99, "status": "0",
}, token=token)
check("POST /system/post 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("post add", js)
new_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if new_id:
    created["post"] = new_id
    st, js = req("PUT", "/system/post", {
        "postId": new_id, "postName": "探针岗位已改", "postCode": "probe",
        "postSort": 98, "status": "0",
    }, token=token)
    check("PUT  /system/post 修改", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/post/{new_id}", token=token)
    check("DELETE /system/post/{id}", ok(st, js), f"status={st}")

# 6b. 写操作闭环：字典类型 + 字典数据
st, js = req("POST", "/system/dict/type", {
    "dictName": "探针字典", "dictType": "probe_dict", "remark": "probe",
}, token=token)
check("POST /system/dict/type 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("dict type add", js)
dt_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if dt_id:
    created["dict_type"] = dt_id
    st, js = req("PUT", "/system/dict/type", {
        "dictId": dt_id, "dictName": "探针字典改", "dictType": "probe_dict", "remark": "probe2",
    }, token=token)
    check("PUT  /system/dict/type 修改", ok(st, js), f"status={st}")
    st, js = req("POST", "/system/dict/data", {
        "dictType": "probe_dict", "dictLabel": "探针项", "dictValue": "v1",
        "cssClass": "", "listClass": "info", "dictSort": 1, "remark": "probe",
    }, token=token)
    check("POST /system/dict/data 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
    if not ok(st, js):
        dump("dict data add", js)
    dd_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
    if dd_id:
        created["dict_data"] = dd_id
        st, js = req("PUT", "/system/dict/data", {
            "dictCode": dd_id, "dictType": "probe_dict", "dictLabel": "探针项改",
            "dictValue": "v2", "cssClass": "", "listClass": "success", "dictSort": 2, "remark": "probe2",
        }, token=token)
        check("PUT  /system/dict/data 修改", ok(st, js), f"status={st}")
        st, js = req("DELETE", f"/system/dict/data/{dd_id}", token=token)
        check("DELETE /system/dict/data/{id}", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/dict/type/{dt_id}", token=token)
    check("DELETE /system/dict/type/{id}", ok(st, js), f"status={st}")

# 6c. 写操作闭环：菜单
st, js = req("POST", "/system/menu", {
    "menuName": "探针菜单", "parentId": 0, "orderNum": 99, "path": "probe",
    "component": "system/user/index", "queryParam": "", "menuType": "C",
    "visible": "0", "status": "0", "perms": "system:probe:list", "icon": "#",
}, token=token)
check("POST /system/menu 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("menu add", js)
m_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if m_id:
    st, js = req("PUT", "/system/menu", {
        "menuId": m_id, "menuName": "探针菜单改", "parentId": 0, "orderNum": 98,
        "path": "probe", "component": "system/user/index", "queryParam": "",
        "menuType": "C", "visible": "0", "status": "0", "perms": "system:probe:list", "icon": "#",
    }, token=token)
    check("PUT  /system/menu 修改", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/menu/{m_id}", token=token)
    check("DELETE /system/menu/{id}", ok(st, js), f"status={st}")

# 6d. 写操作闭环：部门
st, js = req("POST", "/system/dept", {
    "deptName": "探针部门", "parentId": 100, "orderNum": 99,
    "leader": 1, "phone": "13800000000", "email": "p@t.io", "status": "0",
}, token=token)
check("POST /system/dept 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("dept add", js)
d_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if d_id:
    st, js = req("PUT", "/system/dept", {
        "deptId": d_id, "deptName": "探针部门改", "parentId": 100, "orderNum": 98,
        "leader": 1, "phone": "13800000001", "email": "p2@t.io", "status": "0",
    }, token=token)
    check("PUT  /system/dept 修改", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/dept/{d_id}", token=token)
    check("DELETE /system/dept/{id}", ok(st, js), f"status={st}")

# 6e. 写操作闭环：参数
st, js = req("POST", "/system/config", {
    "configName": "探针参数", "configKey": "probe.key", "configValue": "v1",
    "configType": "N", "remark": "probe",
}, token=token)
check("POST /system/config 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("config add", js)
c_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if c_id:
    st, js = req("PUT", "/system/config", {
        "configId": c_id, "configName": "探针参数改", "configKey": "probe.key",
        "configValue": "v2", "configType": "N", "remark": "probe2",
    }, token=token)
    check("PUT  /system/config 修改", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/config/{c_id}", token=token)
    check("DELETE /system/config/{id}", ok(st, js), f"status={st}")

# 6f. 写操作闭环：公告
st, js = req("POST", "/system/notice", {
    "noticeTitle": "探针公告", "noticeType": "1", "noticeContent": "probe",
    "status": "0", "remark": "probe",
}, token=token)
check("POST /system/notice 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("notice add", js)
n_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if n_id:
    st, js = req("PUT", "/system/notice", {
        "noticeId": n_id, "noticeTitle": "探针公告改", "noticeType": "1",
        "noticeContent": "probe2", "status": "0", "remark": "probe2",
    }, token=token)
    check("PUT  /system/notice 修改", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/notice/{n_id}", token=token)
    check("DELETE /system/notice/{id}", ok(st, js), f"status={st}")

# 6g. 用户写操作
st, js = req("POST", "/system/user", {
    "userName": "probeuser", "nickName": "探针用户", "deptId": 100,
    "password": "Probe@12345", "email": "probe@u.io", "phonenumber": "13900000000",
    "sex": "0", "status": "0", "roleIds": [2], "postIds": [],
}, token=token)
check("POST /system/user 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("user add", js)
u_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if u_id:
    st, js = req("PUT", "/system/user", {
        "userId": u_id, "userName": "probeuser", "nickName": "探针用户改", "deptId": 100,
        "email": "probe2@u.io", "phonenumber": "13900000001", "sex": "1",
        "status": "0", "roleIds": [2], "postIds": [],
    }, token=token)
    check("PUT  /system/user 修改", ok(st, js), f"status={st}")
    st, js = req("PUT", "/system/user/resetPwd", {"userId": u_id, "password": "Probe@54321"}, token=token)
    check("PUT  /system/user/resetPwd", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/user/{u_id}", token=token)
    check("DELETE /system/user/{id}", ok(st, js), f"status={st}")

# 6h. 角色写操作
st, js = req("POST", "/system/role", {
    "roleName": "探针角色", "roleKey": "probe", "roleSort": 99,
    "status": "0", "menuIds": [1, 100],
}, token=token)
check("POST /system/role 新增", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("role add", js)
r_id = (js.get("data") or 0) if isinstance(js.get("data"), int) else 0
if r_id:
    st, js = req("PUT", "/system/role/changeStatus", {"roleId": r_id, "status": "1"}, token=token)
    check("PUT  /system/role/changeStatus", ok(st, js), f"status={st}")
    st, js = req("DELETE", f"/system/role/{r_id}", token=token)
    check("DELETE /system/role/{id}", ok(st, js), f"status={st}")

# 7. 字典数据按类型 / 参数按 key
st, js = req("GET", "/system/dict/data/type/sys_user_sex", token=token)
check("GET  /system/dict/data/type/{type}", ok(st, js) and isinstance(js.get("data"), list),
      f"status={st} 条数={len(js.get('data') or [])}")
if not ok(st, js):
    dump("dict by type", js)
st, js = req("GET", "/system/config/configKey/sys.user.initPassword", token=token)
check("GET  /system/config/configKey/{key}", ok(st, js), f"status={st} msg={js.get('msg')}")
if not ok(st, js):
    dump("config by key", js)

# 8. 菜单树授权
st, js = req("GET", "/system/menu/roleMenuTreeselect/1", token=token)
check("GET  /system/menu/roleMenuTreeselect/{roleId}", ok(st, js) and "menus" in js, f"status={st}")
if not ok(st, js):
    dump("roleMenuTreeselect", js)

# 9. 越权：无 token 访问列表应 401
st, js = req("GET", "/system/user/list?pageNum=1&pageSize=10")
check("未带 token 访问列表返回 401", st == 401, f"status={st}")

# 9b. 头像：multipart 上传 + 静态回显 + 数据库回写
def upload_avatar(path, token):
    """构造 multipart/form-data 请求，字段名 avatarfile。"""
    boundary = "----probeBoundary7f3a9c1e"
    # 1x1 像素 PNG（最小合法 PNG）
    png = bytes.fromhex(
        "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
        "1f15c4890000000a49444154789c6300010000050001"
        "0d0a2db40000000049454e44ae426082"
    )
    body = b""
    body += ("--%s\r\n" % boundary).encode()
    body += b'Content-Disposition: form-data; name="avatarfile"; filename="probe.png"\r\n'
    body += b"Content-Type: image/png\r\n\r\n"
    body += png + b"\r\n"
    body += ("--%s--\r\n" % boundary).encode()
    url = BASE + path
    r = urllib.request.Request(
        url, data=body, method="POST",
        headers={
            "Content-Type": "multipart/form-data; boundary=%s" % boundary,
            "Authorization": "Bearer " + token,
        },
    )
    try:
        with urllib.request.urlopen(r, timeout=30) as resp:
            return resp.status, json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"msg": raw[:200]}


st, js = upload_avatar("/system/user/profile/avatar", token)
img_url = ((js.get("data") or {}) if isinstance(js.get("data"), dict) else {}).get("imgUrl", "")
check("POST /system/user/profile/avatar 上传", ok(st, js) and img_url.startswith("/profile/avatar/"),
      f"status={st} imgUrl={img_url}")
if not ok(st, js):
    dump("avatar upload", js)

if img_url:
    # 静态回显：顶层路径，不带 token 也应返回图片字节
    root = BASE.rsplit("/manager-api", 1)[0]
    try:
        with urllib.request.urlopen(root + img_url, timeout=20) as resp:
            ct = resp.headers.get("Content-Type", "")
            n = len(resp.read())
        check("GET  头像静态回显", n > 0 and ct.startswith("image/"), f"status=200 {ct} {n} bytes")
    except Exception as e:
        check("GET  头像静态回显", False, f"异常: {e}")

    # 数据库已回写 avatar 字段
    st, js = req("GET", "/system/user/getInfo", token=token)
    db_avatar = ((js.get("data") or {}).get("user") or {}).get("avatar", "")
    check("getInfo 返回头像路径", db_avatar == img_url, f"avatar={db_avatar}")

# 10. 登出
st, js = req("POST", "/auth/logout", token=token)
check("POST /auth/logout", ok(st, js), f"status={st}")

lines.append("")
lines.append(f"合计: 通过 {passed} / 失败 {failed}")
open(OUT, "w", encoding="utf-8").write("\n".join(lines))
print("\n".join(lines))
