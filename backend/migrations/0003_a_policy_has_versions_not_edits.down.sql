DROP TRIGGER IF EXISTS governance_policies_threshold_is_reachable ON governance_policies;
DROP FUNCTION IF EXISTS assert_policy_threshold_is_reachable();
DROP TABLE IF EXISTS policy_signers;
DROP TABLE IF EXISTS governance_policies;
