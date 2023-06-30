#!/bin/bash
set -ex

rm -rf staging/ || true
mkdir staging/
cd staging

git clone --depth 1 --branch main git@github.com:oxidecomputer/omicron
git clone --depth 1 --branch main git@github.com:oxidecomputer/crucible
git clone --depth 1 --branch master git@github.com:oxidecomputer/propolis
git clone --depth 1 --branch main git@github.com:oxidecomputer/dendrite
git clone --depth 1 --branch main git@github.com:oxidecomputer/maghemite
git clone --depth 1 --branch master git@github.com:oxidecomputer/opte

../target/debug/lockstep

