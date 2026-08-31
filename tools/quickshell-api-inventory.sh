#!/bin/sh
set -eu

root=${1:-xtra/quickshell}
src=$root/src

test -d "$src"
printf 'reference\t%s\n' "$(git -C "$root" rev-parse HEAD)"

find "$src" -type f \( -name '*.hpp' -o -name '*.h' \) \
  ! -path '*/test/*' -print |
  LC_ALL=C sort |
  while IFS= read -r file; do
    awk '
      function module_name(path, value) {
        value = path
        sub(/^.*\/src\//, "", value)
        sub(/\/.*/, "", value)
        return value
      }
      function clean(text) {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", text)
        gsub(/[[:space:]]+/, " ", text)
        return text
      }
      function emit(kind, owner, declaration) {
        printf "%s\t%s\t%s\t%s:%d\t%s\n", \
          kind, module_name(FILENAME), owner, FILENAME, FNR, clean(declaration)
      }
      /^[[:space:]]*(class|struct)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/ {
        class_name = $2
        sub(/[:{;].*/, "", class_name)
      }
      /^[[:space:]]*namespace[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/ {
        namespace_name = $2
        sub(/[[:space:]{].*/, "", namespace_name)
      }
      /^[[:space:]]*Q_NAMESPACE/ { class_name = namespace_name }
      /^[[:space:]]*QML_(ELEMENT|NAMED_ELEMENT|SINGLETON|UNCREATABLE|ANONYMOUS)/ {
        emit("qml", class_name, $0)
      }
      /^[[:space:]]*QML_(ATTACHED|EXTENDED|FOREIGN)/ {
        emit("qml-meta", class_name, $0)
      }
      !/^[[:space:]]*#/ && /(Q_PROPERTY|QSDOC_PROPERTY_OVERRIDE)/ {
        emit("property", class_name, $0)
      }
      /Q_INVOKABLE/ {
        emit("method", class_name, $0)
      }
      /^[[:space:]]*Q_ENUM(_NS)?/ {
        emit("enum", class_name, $0)
      }
      /^[[:space:]]*signals:/ { in_signals = 1; next }
      /^[[:space:]]*(public|private|protected)( slots)?:/ { in_signals = 0 }
      in_signals && /^[[:space:]]*[A-Za-z_][A-Za-z0-9_:<>, *&]*\([^;]*\);[[:space:]]*$/ {
        emit("signal", class_name, $0)
      }
    ' "$file"
  done

find "$src" -type f -name '*.qml' ! -path '*/test/*' -print |
  LC_ALL=C sort |
  while IFS= read -r file; do
    awk '
      function module_name(path, value) {
        value = path
        sub(/^.*\/src\//, "", value)
        sub(/\/.*/, "", value)
        return value
      }
      function clean(text) {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", text)
        gsub(/[[:space:]]+/, " ", text)
        return text
      }
      function emit(kind, declaration) {
        printf "%s\t%s\t%s\t%s:%d\t%s\n", \
          kind, module_name(FILENAME), qml_name, FILENAME, FNR, clean(declaration)
      }
      FNR == 1 {
        qml_name = FILENAME
        sub(/^.*\//, "", qml_name)
        sub(/\.qml$/, "", qml_name)
        emit("qml-file", qml_name)
      }
      /^[[:space:]]*(readonly[[:space:]]+)?property[[:space:]]/ {
        emit("property", $0)
      }
      /^[[:space:]]*signal[[:space:]]/ { emit("signal", $0) }
      /^[[:space:]]*function[[:space:]]/ { emit("method", $0) }
    ' "$file"
  done
