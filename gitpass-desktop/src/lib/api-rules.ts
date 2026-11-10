import { invoke } from "@tauri-apps/api/core";

export type AccountRuleRow = {
  id: string;
  account_id: string;
  path_prefix: string | null;
  github_owner: string | null;
  label: string | null;
};

export type AccountRuleUpsert = {
  id?: string | null;
  account_id: string;
  path_prefix?: string | null;
  github_owner?: string | null;
  label?: string | null;
};

export const rulesApi = {
  list: () => invoke<AccountRuleRow[]>("account_rules_list"),
  upsert: (rule: AccountRuleUpsert) => invoke<AccountRuleRow[]>("account_rules_upsert", { rule }),
  remove: (ruleId: string) => invoke<AccountRuleRow[]>("account_rules_remove", { ruleId }),
};
