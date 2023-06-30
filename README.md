# lockstep

This tool will print out the steps necessary to keep the crucible, propolis,
and omicron repositories using the same revisions. Specifically, it will:

- look into Cargo.toml for each project, and print out the changes needed to use
  the same git revisions
- look into omicron's package-manifest.toml, and print out the changes needed to
  use the correct git revision and the expected image digest.

lockstep reads information from the local filesystem, not from git remotes, and
as a result requires checking out all of the repositories into the same
directory and running the tool from that directory.

for example, if propolis was updated, lockstep will print something like:
```
update ./omicron/sled-agent/Cargo.toml propolis rev from ec4f3a41a638ea6c3316a86f30f1895f4877f2ef to eaec980e060b368c4ca39aaaaf7757cecdb43ecc
```

another example: all of the Cargo.toml values are correct but omicron's
package-manifest.toml needs updating:
```
update omicron package manifest crucible sha256 from 9f73687e4d883a7277af6655e77026188144ada144e4243c90cc139a9a9df6d7 to 174856320e151aeeb12c595392c2289934a0345f669126297cce9ca7249099e3
update omicron package manifest crucible rev from 257032d1e842901d427f344a396d78b9b85b183f to cb363bcb1976093437be33d0160667cd89e53611
wait for propolis-server image for 47ef18a5b0eb7a208ae43e669cf0a93d65576114 to be built (reqwest returned 500 Internal Server Error)
```

lockstep will also look into Cargo.lock to check for outdated revisions there.

if nothing is required, lockstep won't print anything.

## TODO
