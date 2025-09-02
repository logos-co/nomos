#!/bin/bash
# -----------------------------------------------------------------------------
# Script Name : add-circuits-env.sh
# Description : Extracts circuit archives, finds the binaries, and exports environment variables for GitHub Actions.
# Requirements: Downloaded .tar.gz archives are expected to follow a couple of conventions:
# - The archives should be located in the specified directory.
# - The archive names should start with the application name (e.g., "pol-", "prover-", "verifier-").
# - The binaries inside the archives should match the application names.
# Usage       : ./add-circuits-env.sh <CIRCUITS_DIRECTORY>
# -----------------------------------------------------------------------------

set -euo pipefail

# Process Arguments
CIRCUITS_DIRECTORY="$1"

if [[ -z "$CIRCUITS_DIRECTORY" ]]; then
  echo "Usage: $0 <CIRCUITS_DIRECTORY>"
  exit 1
fi

if [ ! -d "$CIRCUITS_DIRECTORY" ]; then
  echo "Error: Circuits directory '$CIRCUITS_DIRECTORY' does not exist." >&2
  exit 2
fi

# Change to the circuits directory
cd "$CIRCUITS_DIRECTORY" || exit 3

# Initialize arrays for error handling
NOT_FOUND_APP=()

# Process archives
shopt -s nullglob  # Ignore non-matching globs
archives=(*.tar.gz)

if [ ${#archives[@]} -eq 0 ]; then
    echo "No .tar.gz archives found in '$CIRCUITS_DIRECTORY'" >&2
    exit 4
fi

for archive in "${archives[@]}"; do
    echo "Processing $archive..."

    # Get relevant information from the archive name
    directory="${archive%.tar.gz}"
    app=$(basename "$archive" | cut -d'-' -f1)

    # Extract
    tar -xvf "$archive"

    # Get the inner binary path
    binary_full_path=$(find "$directory" -type f -name "$app" -print -quit)
    if [ -z "$binary_full_path" ]; then
        NOT_FOUND_APP+=("$app")
        continue
    fi
    binary_full_path=$(realpath "$binary_full_path")

    # Assign the binary path to an environment variable
    # The environment variable name is dynamically constructed
    # by converting the app name to uppercase and prefixing it with "NOMOS_<APP_NAME>"
    # E.g.: pol -> NOMOS_POL
    env_var_name="NOMOS_$(echo "$app" | tr '[:lower:]' '[:upper:]')"
    echo "$env_var_name=$binary_full_path" >> "$GITHUB_ENV"
done

# If any applications could not be found, print a warning and exit
if [ ${#NOT_FOUND_APP[@]} -ne 0 ]; then
    echo "Warning: Not found apps: ${NOT_FOUND_APP[*]}" >&2
    exit 5
fi
