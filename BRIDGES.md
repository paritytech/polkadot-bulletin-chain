# Using Parity Bridges Common dependency (`git subtree`).

In `./bridges` sub-directory you can find a `git subtree` imported version of:
[parity-bridges-common](https://github.com/paritytech/parity-bridges-common/) repository.

# How to pull latest Bridges code to the `bridges` subtree
(in practice)

The `bridges` repo has a stabilized branch `polkadot-v.1.0.0-audited` dedicated for releasing
and based on Polkadot v1.0.0 code.

```
cd <polkadot-bulletin-chain-git-repo-dir>

# needs to be done only once
git remote add -f bridges https://github.com/paritytech/parity-bridges-common.git

# this will update new git branches from bridges repo
# there could be unresolved conflicts, but dont worry,
# lots of them are caused because of removed unneeded files with patch step,
git fetch bridges --prune
git subtree pull --prefix=bridges bridges polkadot-v.1.0.0-audited --squash

# if the command above fails with the "Can't squash-merge: 'bridges' was never added" or
# "fatal: refusing to merge unrelated histories" error, use this:
# git merge -s subtree -Xsubtree="bridges" bridges/polkadot-v.1.0.0-audited --allow-unrelated-histories --squash

# so, after fetch and before solving conflicts just run patch,
# this will remove unneeded files and checks if subtree modules compiles
#
# if it fails to build, you'll need to resolve conflicts manually
./bridges/scripts/verify-pallets-build.sh --ignore-git-state --no-revert

# if there are conflicts, this could help,
# this removes locally deleted files at least (move changes to git stash for commit)
git status -s | awk '$1 == "DU" || $1 == "D" || $1 == "MD" || $1 == "AD" {print $2}' | grep "^bridges/" | xargs git rm -q --ignore-unmatch

# (optional) when conflicts resolved, you can check build again - should pass
# also important: this updates global Cargo.lock
git commit --amend -S -m "updating bridges subtree + remove extra folders"

# add changes to the commit, first command `fetch` starts merge,
# so after all conflicts are solved and patch passes and compiles,
# then we need to finish merge with:
git merge --continue
````
