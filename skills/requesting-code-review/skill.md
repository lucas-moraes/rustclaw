---
name: requesting-code-review
description: Use when completing tasks, implementing major features, or before merging to verify work meets requirements
---

# Requesting Code Review

Thorough self-review before considering work complete catches issues early. Review your changes systematically to ensure quality and completeness.

**Core principle:** Review early, review often.

## When to Request Review

**Mandatory:**
- After completing major feature
- Before merge to main
- After each significant task

**Optional but valuable:**
- When stuck (fresh perspective from user)
- Before refactoring (baseline check)
- After fixing complex bug

## How to Review Your Work

**1. Get git SHAs:**
```bash
BASE_SHA=$(git rev-parse HEAD~1)  # or origin/main
HEAD_SHA=$(git rev-parse HEAD)
```

**2. Review the changes systematically:**

- Check what was implemented vs requirements
- Verify tests cover the changes
- Check for code quality issues
- Look for edge cases or potential bugs
- Ensure documentation is updated

**3. Act on findings:**
- Fix Critical issues immediately
- Fix Important issues before proceeding
- Note Minor issues for later

## Example

```
[Just completed Task 2: Add verification function]

You: Let me review the changes before proceeding.

BASE_SHA=$(git log --oneline | grep "Task 1" | head -1 | awk '{print $1}')
HEAD_SHA=$(git rev-parse HEAD)

git diff $BASE_SHA..$HEAD_SHA

Review findings:
  Strengths: Clean architecture, real tests
  Issues:
    Important: Missing progress indicators
    Minor: Magic number (100) for reporting interval

[Fix progress indicators]
[Continue to Task 3]
```

## Integration with Workflows

**Task-Based Development:**
- Review after EACH major task
- Catch issues before they compound
- Fix before moving to next task

**Ad-Hoc Development:**
- Review before merge
- Review when uncertain

## Red Flags

**Never:**
- Skip review because "it's simple"
- Ignore Critical issues
- Proceed with unfixed Important issues
