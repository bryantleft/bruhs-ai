# shellcheck shell=bash
# Shared by every bench/tasks/<name>/check.sh. Source it, then call
# `run_hidden_tests <task-dir> <workdir>`.
#
# The hidden tests (`<task-dir>/hidden/*.py`) and pristine copies of the
# repo's own visible tests (`<task-dir>/repo/tests/test_*.py`, so an agent
# cannot pass by weakening them) go into `<workdir>/_bench_hidden/`, which is
# run with unittest from the workdir root. Only called after the agent
# finished, so the agent never sees the hidden tests.

run_hidden_tests() {
  local task_dir="$1" work="$2" dest
  dest="$work/_bench_hidden"
  rm -rf "$dest"
  mkdir -p "$dest"
  : >"$dest/__init__.py"
  cp "$task_dir"/hidden/*.py "$dest/"
  if [ -d "$task_dir/repo/tests" ]; then
    find "$task_dir/repo/tests" -maxdepth 1 -name 'test_*.py' -exec cp {} "$dest/" \;
  fi
  (cd "$work" && PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s _bench_hidden -t . 2>&1)
}

# Fails (returns 1) when an existing template file other than the allowed
# ones (paths relative to repo/; glob patterns such as 'tests/*' work) was
# modified or deleted. New files are
# ignored, as are caches, git metadata and the hidden test dir.
only_changed() {
  local task_dir="$1" work="$2" rel bad=0 allowed keep
  shift 2
  while IFS= read -r rel; do
    rel="${rel#./}"
    allowed=0
    for keep in "$@"; do
      # shellcheck disable=SC2254 # $keep is a glob pattern on purpose
      case "$rel" in $keep) allowed=1 ;; esac
    done
    [ "$allowed" = 1 ] && continue
    if ! cmp -s "$task_dir/repo/$rel" "$work/$rel"; then
      echo "unexpected change: $rel"
      bad=1
    fi
  done < <(cd "$task_dir/repo" && find . -type f ! -path '*/__pycache__/*' ! -name '.DS_Store')
  return "$bad"
}
