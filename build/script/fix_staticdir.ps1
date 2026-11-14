# fix_staticdir.ps1
# Rewrite static_dir in manager.xml for the installed layout.
# install.bat calls this with two (or more) file paths as arguments.
#
# Source config ships static_dir="../ui/dist" or "manager/ui/dist".
# The installed frontend lives in INSTALL_DIR\web, so rewrite to "web".

# PowerShell 5.1 compat: [System.Text.UTF8Encoding]::new($false) works;
# New-Object with boolean ctor is not reliable on older runtimes.
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

foreach ($f in $args) {
    if (Test-Path $f) {
        $content = [System.IO.File]::ReadAllText($f)
        # Match static_dir="..."  -- allow optional spaces around '='
        $modified = [regex]::Replace($content, 'static_dir\s*=\s*"[^"]*"', 'static_dir="web"')
        if ($modified -ne $content) {
            [System.IO.File]::WriteAllText($f, $modified, $utf8NoBom)
            Write-Host "Fixed static_dir -> web : $f"
        } else {
            Write-Host "static_dir already web  : $f"
        }
    } else {
        Write-Host "SKIP (not found)       : $f"
    }
}
