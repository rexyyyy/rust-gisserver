import importlib.util, json, os, random, sys

spec = importlib.util.spec_from_file_location(
    "valmod", r"D:\mycomputor\documents\gis-wt\_validate_3dtiles.py")
val = importlib.util.module_from_spec(spec)
spec.loader.exec_module(val)

tdir = r"D:\mycomputor\documents\gis-wt\.testdata\3dtiles\osgb4326_3dtiles"
d = json.load(open(os.path.join(tdir, "tileset.json"), encoding="utf-8"))
tiles = list(val.find_tiles(d, tdir))
random.seed(42)
sample = random.sample(tiles, min(300, len(tiles)))
ok = bad = with_tex = 0
bad_samples = []
for p in sample:
    b = open(p, "rb").read()
    r, info = val.parse_gltf_json(b)
    if r:
        ok += 1
        if info.get("has_POSITION"):
            pass
        # texture evidence: glTF text scan for images/textures
        if b'"images"' in b or b'"textures"' in b:
            with_tex += 1
    else:
        bad += 1
        if len(bad_samples) < 5:
            bad_samples.append((os.path.relpath(p, tdir), info))
print("total_tiles=%d sampled=%d PARSE_OK=%d PARSE_BAD=%d with_images_or_textures=%d"
      % (len(tiles), len(sample), ok, bad, with_tex))
for rel, info in bad_samples:
    print("  BAD", rel, info)
print("RESULT:", "SAMPLE_ALL_VALID" if bad == 0 and ok > 0 else "SAMPLE_HAS_BAD")
