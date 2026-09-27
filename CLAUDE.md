# fohmixer — Project Instructions

## Overview

TODO: jednou vetou čo projekt je a pre koho.

## Branch policy

Two-branch: work na `dev`, PR `dev`→default. (Ak projekt používa inú konvenciu, uprav tento riadok — onboarding default branch nikdy nemení.)

## Playbook router

- <area> → `.claude/rules/<area>.md` (auto-loads on its `paths:`)
- build / deploy / release → load `.claude/skills/<area>` (invoke by name)
