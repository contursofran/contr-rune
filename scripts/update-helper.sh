#!/bin/sh
# The updater's helper (embedded in the app, run detached by /bin/sh -c; the args follow as $1..$4).
#   $1  pid of the running app: wait until it has exited
#   $2  the new contr-rune.app (unzipped by the app)
#   $3  the installed contr-rune.app to replace
#   $4  the work folder to delete at the end
# The old app stays in place if anything fails before the swap, and is opened again either way.
pid=$1
new=$2
app=$3
work=$4
tries=0
while kill -0 "$pid" 2>/dev/null; do
  tries=$((tries + 1))
  [ "$tries" -gt 300 ] && break
  sleep 0.1
done
if [ "$tries" -le 300 ]; then
  staged="$app.new.$$"
  old="$app.old.$$"
  rm -rf "$staged" "$old"
  if /usr/bin/ditto "$new" "$staged" && mv "$app" "$old"; then
    if mv "$staged" "$app"; then
      rm -rf "$old"
    else
      mv "$old" "$app"
    fi
  fi
  rm -rf "$staged"
  /usr/bin/xattr -dr com.apple.quarantine "$app" 2>/dev/null
fi
/usr/bin/open "$app"
rm -rf "$work"
