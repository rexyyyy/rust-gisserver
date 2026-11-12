import io
import sys

# 从 desktop/config.js 的 THREED_TILES_SERVICES 中按 id 删除一个条目对象
p = r"d:\mycomputor\documents\gis-wt\desktop\src\config.js"
target_id = sys.argv[1] if len(sys.argv) > 1 else "osgb4326"

src = io.open(p, encoding="utf-8").read()
start_marker = "  {\n    id: '%s',\n" % target_id
end_marker = "\n  },\n"

i = src.find(start_marker)
if i == -1:
    print("entry id=%s not found, no change" % target_id)
else:
    j = src.find(end_marker, i + len(start_marker))
    if j == -1:
        print("END MARKER NOT FOUND")
    else:
        block = src[i:j + len(end_marker)]
        src = src[:i] + src[j + len(end_marker):]
        io.open(p, "w", encoding="utf-8", newline="\n").write(src)
        print("removed entry id=%s:\n%s" % (target_id, block))
