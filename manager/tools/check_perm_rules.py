"""校验 PERM_RULES 模式表：模拟匹配，检查规则覆盖与冲突。

要点：
1. 表内每条规则自身必须能匹配自己的模式（`:x` 段除外）；
2. 同一 (方法, 路径) 不能被两条规则同时匹配（按表顺序取第一条）；
3. 对每条规则，构造一个具体实例路径，确认匹配到的第一条规则就是它自己。
"""
import re
import sys
from pathlib import Path

# 相对当前仓库定位源码，避免绑定某台机器的绝对路径
SRC_ROOT = Path(__file__).resolve().parents[1] / "backend" / "src"

RULES = []
for line in open(SRC_ROOT / "perm.rs", encoding="utf-8"):
    m = re.match(r'\s*\("(GET|POST|PUT|DELETE)",\s*"([^"]+)",\s*"([^"]+)"\),', line)
    if m:
        RULES.append((m.group(1), m.group(2), m.group(3)))


def match_pattern(pattern, path):
    ps = [s for s in pattern.split("/") if s]
    xs = [s for s in path.split("/") if s]
    if len(ps) != len(xs):
        return False
    # 通配段：axum 0.7 的 ":x" 与 0.8 的 "{x}" 都视为占位符
    return all(p.startswith(":") or (p.startswith("{") and p.endswith("}")) or p == x
               for p, x in zip(ps, xs))


def first_match(method, path):
    for meth, pat, perm in RULES:
        if meth == method and match_pattern(pat, path):
            return pat, perm
    return None, None


print(f"规则条数：{len(RULES)}")
print()
print("=== 1) 冲突检查：同 (方法,路径) 被多条规则匹配 ===")
conflicts = 0
for i, (meth, pat, perm) in enumerate(RULES):
    # 用具体实例替换 :x
    concrete = re.sub(r"(:\w+|\{\w+\})", "1", pat)
    hit_pat, hit_perm = first_match(meth, concrete)
    if hit_pat != pat:
        # 找出所有能匹配的规则
        allhits = [(p, q) for (m, p, q) in RULES if m == meth and match_pattern(p, concrete)]
        print(f"  ✗ {meth} {concrete}")
        print(f"      期望 {pat} ({perm})  实际先命中 {hit_pat} ({hit_perm})")
        print(f"      全部候选：{allhits}")
        conflicts += 1
if not conflicts:
    print("  ✓ 无冲突：每条规则的实例路径都首先命中自己")

print()
print("=== 2) 覆盖率检查：本项目实际注册的所有端点 ===")
# 从 handlers/mod.rs 提取路由
src = open(SRC_ROOT / "handlers" / "mod.rs", encoding="utf-8").read()
# 无权限注解的端点（NO_PERM_PATTERNS）
NO_PERM = [
    ("GET", "/system/user/profile"),
    ("PUT", "/system/user/profile"),
    ("PUT", "/system/user/profile/updatePwd"),
    ("POST", "/system/user/profile/avatar"),
    ("GET", "/monitor/online"),
    ("DELETE", "/monitor/online/myself"),
    ("DELETE", "/monitor/online/myself/:tokenId"),
    ("GET", "/system/config/configKey/:configKey"),
    ("GET", "/system/dict/data/type/:dictType"),
]
# 白名单（无需登录）
WHITE = ["/auth/login", "/auth/code", "/captchaImage", "/health"]

missing = []
for path, methods in re.findall(r'\.route\("([^"]+)"\s*,\s*(.*?)\)\s*(?:\.route|\.layer)', src, re.S):
    mm = re.findall(r'\b(get|post|put|delete)\(', methods)
    if not mm:
        continue
    for meth in set(mm):
        m2 = meth.upper()
        pat, perm = first_match(m2, path)
        if perm is not None:
            continue
        # NO_PERM_PATTERNS 覆盖？
        if any(mm2 == m2 and match_pattern(p, path) for mm2, p in NO_PERM):
            continue
        # 白名单？
        if path in WHITE:
            continue
        missing.append((m2, path))
for m2, path in missing:
    print(f"  ? {m2:6} {path}")
if not missing:
    print("  ✓ 全部命中（PERM_RULES / NO_PERM_PATTERNS / 白名单）")

print()
print("=== 3) 关键语义抽查（权限注解模式）===")
CASES = [
    ("GET", "/system/user/list", "system:user:list"),
    ("GET", "/system/user/1", "system:user:query"),
    ("DELETE", "/system/user/1", "system:user:remove"),
    ("POST", "/system/user", "system:user:add"),
    ("PUT", "/system/user", "system:user:edit"),
    ("GET", "/system/post/list", "system:post:list"),
    ("DELETE", "/system/post/7", "system:post:remove"),
    ("GET", "/system/dict/type/list", "system:dict:list"),
    ("GET", "/system/dict/data/list", "system:dict:list"),
    ("DELETE", "/system/config/1", "system:config:remove"),
    ("GET", "/monitor/online/list", "monitor:online:list"),
    ("DELETE", "/monitor/online/abc123", "monitor:online:forceLogout"),
    ("GET", "/monitor/logininfor/unlock/admin", "monitor:logininfor:unlock"),
    ("DELETE", "/monitor/operlog/clean", "monitor:operlog:remove"),
    ("GET", "/system/menu/treeselect", "system:menu:query"),
    ("DELETE", "/system/menu/cascade/5", "system:menu:remove"),
]
bad = 0
for meth, path, expect in CASES:
    pat, perm = first_match(meth, path)
    ok = perm == expect
    if not ok:
        bad += 1
    print(f"  {'✓' if ok else '✗'} {meth:6} {path:40} -> {perm} (期望 {expect})")

print()
print("结果：冲突", conflicts, "／ 未覆盖端点", len(missing), "／ 语义错", bad)
sys.exit(1 if (conflicts or bad) else 0)
