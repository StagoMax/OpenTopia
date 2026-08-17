-- Version 20 activates the verified migration framework. The ledger table is
-- created by the verified v19 baseline transaction, so this step intentionally
-- performs no structural mutation. Every formal migration remains an immutable
-- checksummed resource, including this framework boundary.
SELECT 1;
