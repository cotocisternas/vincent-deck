# Vincent Deck

A physical control surface for the user's Omarchy desktop, with theme-following
appearance and displays of current desktop state.

## Language

**Action**:
A named desktop control, such as Terminal, Volume, or Workspace, with defined
interactions and appearance.

**Action instance**:
One placement of an action on the deck. Multiple instances of the same action
share the same meaning and live state.

**Key tile**:
The picture shown on one physical key.

**Dial panel**:
The section of the touch strip associated with one dial, showing that action's
live information.

**Theme-following**:
The deck's appearance tracks the current desktop palette. A monochrome desktop
theme produces monochrome deck accents as well.

**Stale state**:
A last-known value whose current accuracy cannot be confirmed. It must be visibly
distinguished from current state, and never implies confirmed off, idle, or muted.

**Full rollback**:
Restoration of the previous working deck setup, including its appearance and
behavior, even after migration cleanup.

**Pending action**:
An action whose command is still in progress. Pending does not confirm that the
requested desktop state has been reached.
