#!/bin/bash
set -e

# Brings in: ROOT_DIR, EXAMPLES_DIR, LIB_DIR, CODEGEN_DIR, CONTRIB_DIR, DOC_DIR
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
source $SCRIPT_DIR/config.sh

# Add Cargo to PATH.
export PATH=${HOME}/.cargo/bin:${PATH}

# Checks that the versions for Cargo projects $@ all match
function check_versions_match() {
  local last_version=""
  for dir in $@; do
    local cargo_toml="${dir}/Cargo.toml"
    if ! [ -f "${cargo_toml}" ]; then
      echo "Cargo configuration file '${cargo_toml}' does not exist."
      exit 1
    fi

    local version=$(grep version ${cargo_toml} | head -n 1 | cut -d' ' -f3)
    if [ -z "${last_version}" ]; then
      last_version="${version}"
    elif ! [ "${version}" = "${last_version}" ]; then
      echo "Versions differ in '${cargo_toml}'. ${version} != ${last_version}"
      exit 1
    fi
  done
}

# Ensures there are not tabs in any file in the directories $@.
function ensure_tab_free() {
  local tab=$(printf '\t')
  local matches=$(grep -I -R "${tab}" $ROOT_DIR | egrep -v '/target|/.git|LICENSE')
  if ! [ -z "${matches}" ]; then
    echo "Tab characters were found in the following:"
    echo "${matches}"
    exit 1
  fi
}

function bootstrap_examples() {
  for file in ${EXAMPLES_DIR}/*; do
    if [ -d "${file}" ]; then
      bootstrap_script="${file}/bootstrap.sh"
      if [ -x "${bootstrap_script}" ]; then
        echo "    Bootstrapping ${file}..."

        env_vars=$(${bootstrap_script})
        bootstrap_result=$?
        if [ $bootstrap_result -ne 0 ]; then
          echo "    Running bootstrap script (${bootstrap_script}) failed!"
          exit 1
        else
          eval $env_vars
        fi
      fi
    fi
  done
}

echo ":: Ensuring all crate versions match..."
check_versions_match "${LIB_DIR}" "${CODEGEN_DIR}" "${CONTRIB_DIR}"

echo ":: Checking for tabs..."
ensure_tab_free

echo ":: Updating dependencies..."
cargo update

echo ":: Boostrapping examples..."
bootstrap_examples

echo ":: Building and testing libraries..."
cargo test --all-features --all
