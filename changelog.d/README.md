# Changelog fragments

A pull request records its changelog entry here, in a file of its own, and
never edits `CHANGELOG.md`. Two pull requests then never conflict over the
changelog, so a pull request whose code doesn't overlap main can merge without
another merge of main.

## Adding an entry

Add one file, `changelog.d/<name>.md`, holding exactly the bullet(s) you would
have put under the Unreleased heading of `CHANGELOG.md`, in the same style:

```markdown
- Swim in slime and Velious water as the Titanium program does (checklist
  6.483d).
  - **Breaking:** ...
```

Name the file after the checklist label and a short slug, for example
`6.650-ship-deck-collision.md`, or a short slug alone when there is no label.
The name only has to be unique. To change your entry later, edit your own
fragment. Don't edit another pull request's fragment.

## Folding

From time to time (and before a release) a maintainer folds every fragment into
`CHANGELOG.md`, newest first, in a pull request of its own:

```console
python3 contrib/fold-changelog.py
```

Until then, entries that are merged but not yet folded are the files in this
directory.
