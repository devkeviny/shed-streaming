-- Licensed under the GNU Affero General Public License v3.0 — see LICENSE.

-- Per-layer visibility: a hidden layer stays in the scene (and in the editor)
-- but is skipped by the compositor. Lets the operator show/hide overlays
-- without deleting and re-adding them.
ALTER TABLE scene_layers ADD COLUMN visible INTEGER NOT NULL DEFAULT 1;
