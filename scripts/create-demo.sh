#!/usr/bin/env bash
set -e

mkdir "${1:-demo-files}"
cd "${1:-demo-files}"
git init -b master

mkdir foo
printf 'im a file\n' > readme.md
printf "%s\n" "console.log('hello world');" > foo/program.js
git add .
git -c commit.gpgsign=false commit -m first

cat > foo/program.js <<'JS'
// This is a very serious project!
console.log('hello world');
console.log('11' + 22);

JS
printf 'console.log(123);' >> foo/program.js

mkdir fee
printf 'im a file too\n' > fee/test.txt
