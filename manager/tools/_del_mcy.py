import io

p = r"d:\mycomputor\documents\gis-wt\desktop\src\config.js"
src = io.open(p, encoding="utf-8").read()

start_marker = "  {\n    id: 'mcy',\n"
end_marker = "\n  },\n"

i = src.find(start_marker)
if i == -1:
    print("mcy block not found, no change")
else:
    j = src.find(end_marker, i + len(start_marker))
    if j == -1:
        print("END MARKER NOT FOUND")
    else:
        block = src[i:j + len(end_marker)]
        if "3dtiles/mcy/tileset.json" not in block:
            print("safety: block does not contain mcy url, abort")
            print(repr(block))
        else:
            src = src[:i] + src[j + len(end_marker):]
            io.open(p, "w", encoding="utf-8", newline="\n").write(src)
            print("removed mcy block:\n" + block)
