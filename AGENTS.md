# Notes for coding agents

## Required release-note style

Whenever preparing or publishing an app update to this repository, write its GitHub release description in the style below. Prepare the description before publishing; do not rely on an automatically generated commit list. A normal code push does not require creating a release.

- Write release notes in English, even when the conversation is in another language.
- Start the release body with exactly `## What's new?`, followed by a blank line and Markdown bullets. Use this heading once per release.
- Describe actual changes in Flimmer (Windows app and Android app) since the previous app release. Check the changes and validation results before making claims.
- Use short, concrete, user-facing descriptions. Prefer `Area: Change` after the emoji, such as `Downloads: Fixed ...`.
- Order entries: breaking notices, new features, improvements, fixes, maintenance/dependencies. Omit categories with no changes; do not add category headings.
- Emoji meanings are mandatory and consistent across releases. Classify the actual change first; never choose emojis for decoration, variety, or based on the feature's size. Do not substitute other emojis.
- Use these markers according to the change:
  - `📝 BREAKING:` for compatibility changes requiring user action; explain the action.
  - `🎉` for every new user-facing feature or capability, including small features. If users could not do it before and now can, use `🎉`.
  - `⚡️` only for improvements to existing working features, including performance, usability, and reliability improvements. Do not use it for new capabilities or bug fixes.
  - `🦺` for bug fixes and regressions: restoring behavior that was broken or incorrect. Use it even when the fix also makes the app faster or more reliable.
  - `🏗️` for restructuring, removals, and other maintenance changes worth mentioning; add `BREAKING:` when applicable.
  - `⬆️` for dependency updates. Use `⬆️ Updated dependencies` only if dependencies actually changed.
- For a breaking change, keep the appropriate change marker and add `BREAKING:` (for example `🏗️ BREAKING:` for an incompatible removal). Use `📝 BREAKING:` for a separate migration/action notice. Explain what users must do.
- Split changes with different meanings into separate bullets rather than assigning an arbitrary emoji to a mixed entry. Before publishing, check that every bullet's emoji matches its actual change; do not force unused categories into the release.
- Use indented sub-bullets only when a larger change needs useful details.
- Add `[#123](https://github.com/Lampe332/flimmer/issues/123)` or the corresponding `/pull/123` link only when it is real and relevant. Omit links when none exist; never invent issue numbers.
- Avoid raw commit lists, internal testing logs, unsupported claims, and generic filler. Preserve uncertainty when a fix is only a potential fix.
- After the bullets, a release may add a `## Downloads` section that names each attached file (Windows installer, Android APK) and what it is for.
- When a `CHANGELOG.md` exists, use the same English bullet style for new entries under the existing version heading. Keep historical entries intact.

### Format example (illustrative only; do not publish unchanged)

```markdown
## What's new?

- 🎉 Downloads: Added an offline download queue
  - Pause and resume queued episodes
- ⚡️ Downloads: Improved download speed for large files
- 🦺 Player: Fixed playback position not being restored
- ⬆️ Updated dependencies
```
