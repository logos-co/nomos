# Building `nomos-research` with Jenkins

This is a short introduction for developers on how to use `ci` folder to update build dependencies or to modify the build process.

## ci/Dockerfile (Docs, linux target)

Dockerfile is used when `nomos-research` documentation is being built and to lint/test/build for linux target. Official rust image is used with a predefined version. In addition, golang and cargo components are downloaded when the image is being built. `ghp-import` dependency is added for pushing documentation back to the github repository in the `gh-pages` branch.
In general, this file should be used just for defining dependencies. Related steps and build commands for docs and linux targets should be defined in `ci/Jenkinsfile.prs.linux`.

## ci/Jenkinsfile.prs.docs

This file contains the configuration required for Jenkins github user to be able to push the docs to the `gh-pages` branch. It also defines steps for building the documentation and publishing it.

## ci/Jenkinsfile.prs.linux

Two most important places in this file are `environment` and `stages`.
* `environment` - variables defined here will be accessible to every stage that runs on an image built from the `ci/Dockerfile`
* `stages` - used to group shell commands that are related to different steps and their layout reflects in the build job summary.

## ci/Jenkinsfile.prs.macos

Same as in `Jenkinsfile.prs.macos` the only difference is that instead of Docker image, macos is using `shell.nix` to build a shell with all dependencies. The steps defined here should be identical or similar to what's defined in linux file, just instead of running those commands straight in `sh`, use `nix.shell('command')` wrapper.

## shell.nix

Configuration file for the Nix package manager. It defines the build dependencies for `macos` target and can be used to manage and update the dependencies similarly to Dockerfile.
