#!/bin/sh
set -eu

root=${1:-xtra/quickshell}
src=$root/src

test -d "$src"
printf 'reference\t%s\n' "$(git -C "$root" rev-parse HEAD)"

find "$src/core" "$src/io" "$src/widgets" "$src/window" "$src/windowmanager" \
  -type f \( -name '*.hpp' -o -name '*.h' \) -print |
  LC_ALL=C sort |
  while IFS= read -r file; do
    awk '
      function clean(text) {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", text)
        gsub(/[[:space:]]+/, " ", text)
        return text
      }
      /^(class|struct)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/ {
        class_name = $2
        sub(/[:{;].*/, "", class_name)
      }
      /^[[:space:]]*QML_(ELEMENT|NAMED_ELEMENT|SINGLETON|UNCREATABLE|ANONYMOUS)/ {
        printf "qml\t%s\t%s:%d\t%s\n", class_name, FILENAME, FNR, clean($0)
      }
      /(Q_PROPERTY|QSDOC_PROPERTY_OVERRIDE)/ {
        printf "property\t%s\t%s:%d\t%s\n", class_name, FILENAME, FNR, clean($0)
      }
      /Q_INVOKABLE/ {
        printf "method\t%s\t%s:%d\t%s\n", class_name, FILENAME, FNR, clean($0)
      }
      /^[[:space:]]*Q_ENUM/ {
        printf "enum\t%s\t%s:%d\t%s\n", class_name, FILENAME, FNR, clean($0)
      }
      /^[[:space:]]*signals:/ { in_signals = 1; next }
      /^[[:space:]]*(public|private|protected)( slots)?:/ { in_signals = 0 }
      in_signals && /^[[:space:]]*[A-Za-z_][A-Za-z0-9_:<>, *&]*\([^;]*\);[[:space:]]*$/ {
        printf "signal\t%s\t%s:%d\t%s\n", class_name, FILENAME, FNR, clean($0)
      }
    ' "$file"
  done
