#!/usr/bin/env bash
set -e

# Brings in _ROOT, _DIR, _DIRS globals.
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
source "${SCRIPT_DIR}/config.sh"

# Add Cargo to PATH.
export PATH=${HOME}/.cargo/bin:${PATH}
export CARGO_INCREMENTAL=0
CARGO="cargo"

# Checks that the versions for Cargo projects $@ all match
function check_versions_match() {
  local last_version=""
  for dir in "${@}"; do
    local cargo_toml="${dir}/Cargo.toml"
    if ! [ -f "${cargo_toml}" ]; then
      echo "Cargo configuration file '${cargo_toml}' does not exist."
      exit 1
    fi

    local version=$(grep version "${cargo_toml}" | head -n 1 | cut -d' ' -f3)
    if [ -z "${last_version}" ]; then
      last_version="${version}"
    elif ! [ "${version}" = "${last_version}" ]; then
      echo "Versions differ in '${cargo_toml}'. ${version} != ${last_version}"
      exit 1
    fi
  done
}

# Ensures there are no tabs in any file.
function ensure_tab_free() {
  local tab=$(printf '\t')
  local matches=$(git grep -E -I "${tab}" "${PROJECT_ROOT}" | grep -v 'LICENSE')
  if ! [ -z "${matches}" ]; then
    echo "Tab characters were found in the following:"
    echo "${matches}"
    exit 1
  fi
}

# Ensures there are no files with trailing whitespace.
function ensure_trailing_whitespace_free() {
  local matches=$(git grep -E -I "\s+$" "${PROJECT_ROOT}" | grep -v -F '.stderr:')
  if ! [ -z "${matches}" ]; then
    echo "Trailing whitespace was found in the following:"
    echo "${matches}"
    exit 1
  fi
}

function test_contrib() {
  FEATURES=(
    json
    msgpack
    tera_templates
    handlebars_templates
    serve
    helmet
    diesel_postgres_pool
    diesel_sqlite_pool
    diesel_mysql_pool
    postgres_pool
    sqlite_pool
    memcache_pool
    brotli_compression
    gzip_compression
  )

  echo ":: Building and testing contrib [default]..."

  pushd "${CONTRIB_LIB_ROOT}" > /dev/null 2>&1
    $CARGO test $@

    for feature in "${FEATURES[@]}"; do
      echo ":: Building and testing contrib [${feature}]..."
      $CARGO test --no-default-features --features "${feature}" $@
    done
  popd > /dev/null 2>&1
}

function test_core() {
  FEATURES=(
    secrets
    tls
    log
  )

  pushd "${CORE_LIB_ROOT}" > /dev/null 2>&1
    echo ":: Building and testing core [no features]..."
    $CARGO test --no-default-features $@

    for feature in "${FEATURES[@]}"; do
      echo ":: Building and testing core [${feature}]..."
      $CARGO test --no-default-features --features "${feature}" $@
    done
  popd > /dev/null 2>&1
}

function test_examples() {
  echo ":: Building and testing examples..."

  pushd "${EXAMPLES_DIR}" > /dev/null 2>&1
    # Rust compiles Rocket once with the `secrets` feature enabled, so when run
    # in production, we need a secret key or tests will fail needlessly. We
    # ensure in core that secret key failing/not failing works as expected.
    ROCKET_SECRET_KEY="itlYmFR2vYKrOmFhupMIn/hyB6lYCCTXz4yaQX89XVg=" \
      $CARGO test --all $@
  popd > /dev/null 2>&1
}

function test_default() {
  echo ":: Building and testing core libraries..."

  pushd "${PROJECT_ROOT}" > /dev/null 2>&1
    $CARGO test --all --all-features $@
  popd > /dev/null 2>&1
}

function run_benchmarks() {
  echo ":: Running benchmarks..."

  pushd "${BENCHMARKS_ROOT}" > /dev/null 2>&1
    $CARGO bench $@
  popd > /dev/null 2>&1
}

if [[ $1 == +* ]]; then
    CARGO="$CARGO $1"
    shift
fi

# The kind of test we'll be running.
TEST_KIND="default"
KINDS=("contrib" "benchmarks" "core" "examples" "default" "all")

if [[ " ${KINDS[@]} " =~ " ${1#"--"} " ]]; then
    TEST_KIND=${1#"--"}
    shift
fi

echo ":: Preparing. Environment is..."
print_environment
echo "  CARGO: $CARGO"
echo "  EXTRA FLAGS: $@"

echo ":: Ensuring all crate versions match..."
check_versions_match "${ALL_PROJECT_DIRS[@]}"

echo ":: Checking for tabs..."
ensure_tab_free

echo ":: Checking for trailing whitespace..."
ensure_trailing_whitespace_free

echo ":: Updating dependencies..."
if ! $CARGO update ; then
  echo "   WARNING: Update failed! Proceeding with possibly outdated deps..."
fi

case $TEST_KIND in
  core) test_core $@ ;;
  contrib) test_contrib $@ ;;
  examples) test_examples $@ ;;
  default) test_default $@ ;;
  benchmarks) run_benchmarks $@ ;;
  all)
    test_default $@ & default=$!
    test_examples $@ & examples=$!
    test_core $@ & core=$!
    test_contrib $@ & contrib=$!

    failures=()
    if ! wait $default ; then failures+=("ROOT WORKSPACE"); fi
    if ! wait $examples ; then failures+=("EXAMPLES"); fi
    if ! wait $core ; then failures+=("CORE"); fi
    if ! wait $contrib ; then failures+=("CONTRIB"); fi

    if [ ${#failures[@]} -ne 0 ]; then
      tput setaf 1;
      echo -e "\n!!! ${#failures[@]} TEST SUITE FAILURE(S) !!!"
      for failure in "${failures[@]}"; do
        echo "    :: ${failure}"
      done

      tput sgr0
      exit ${#failures[@]}
    fi

    ;;
esac
