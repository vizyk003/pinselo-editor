-- Rename the system account and workspace that own the template library (the seed never updates existing rows).
UPDATE users SET name = 'Pinselo Library' WHERE id = '00000000-0000-4000-8000-00000000c0de';
UPDATE workspaces SET name = 'Pinselo Library' WHERE id = '00000000-0000-4000-8000-00000000c0df';
