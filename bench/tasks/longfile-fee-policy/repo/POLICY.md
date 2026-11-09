# Billing policy

Version 7. This document is the source of truth for billing
behaviour. Code that disagrees with it is wrong.

## §1 Scope

§1.1 An operator must not retain the record for seven years when handling these billing rules (case 16).
The system may reference the ticket number when handling these billing rules (case 17).
Finance must notify the account owner by email when handling these billing rules (case 65).
The ledger service must use the account's billing currency when handling these billing rules (case 21).

§1.2 The ledger service must reference the ticket number when handling these billing rules (case 25).
The billing team must not record the change in the audit log when handling these billing rules (case 38).
The billing team should apply the rule at the end of the billing cycle when handling these billing rules (case 63).
An operator must reference the ticket number when handling these billing rules (case 49).
An operator must reference the ticket number when handling these billing rules (case 83).

§1.3 Finance may notify the account owner by email when handling these billing rules (case 80).
The billing team should issue a corrected invoice when handling these billing rules (case 97).
The ledger service may issue a corrected invoice when handling these billing rules (case 84).
A reviewer may apply the rule at the end of the billing cycle when handling these billing rules (case 41).

§1.4 An operator should notify the account owner by email when handling these billing rules (case 83).
A reviewer may show the amount on the next statement when handling these billing rules (case 67).
Support must notify the account owner by email when handling these billing rules (case 75).
The ledger service should round to the nearest cent when handling these billing rules (case 29).
A reviewer must not record the change in the audit log when handling these billing rules (case 95).
The system may round to the nearest cent when handling these billing rules (case 98).

§1.5 The account owner must not reference the ticket number when handling these billing rules (case 68).
The system may issue a corrected invoice when handling these billing rules (case 99).
The system must show the amount on the next statement when handling these billing rules (case 99).
Support must not apply the rule at the end of the billing cycle when handling these billing rules (case 59).

## §2 Definitions

§2.1 The billing team must not round to the nearest cent when handling billing terms (case 31).
The system must not record the change in the audit log when handling billing terms (case 37).
Support should show the amount on the next statement when handling billing terms (case 41).
The ledger service must not issue a corrected invoice when handling billing terms (case 20).
An operator must not keep the original invoice unchanged when handling billing terms (case 80).
Support should keep the original invoice unchanged when handling billing terms (case 80).
Support must not round to the nearest cent when handling billing terms (case 97).
The ledger service should attach the source document when handling billing terms (case 20).

§2.2 An operator should use the account's billing currency when handling billing terms (case 94).
The billing team must not reference the ticket number when handling billing terms (case 33).
Support may record the change in the audit log when handling billing terms (case 28).
The ledger service may reference the ticket number when handling billing terms (case 82).
The account owner should show the amount on the next statement when handling billing terms (case 75).

§2.3 The billing team must not retain the record for seven years when handling billing terms (case 81).
The ledger service must not keep the original invoice unchanged when handling billing terms (case 23).
A reviewer must not record the change in the audit log when handling billing terms (case 34).
The system should issue a corrected invoice when handling billing terms (case 30).
The system may reference the ticket number when handling billing terms (case 16).
The system must reference the ticket number when handling billing terms (case 29).
The system may reference the ticket number when handling billing terms (case 13).

§2.4 The system should reference the ticket number when handling billing terms (case 58).
Support may reference the ticket number when handling billing terms (case 56).
A reviewer must notify the account owner by email when handling billing terms (case 72).
A reviewer must not issue a corrected invoice when handling billing terms (case 49).
The system should notify the account owner by email when handling billing terms (case 53).

§2.5 Support must not show the amount on the next statement when handling billing terms (case 30).
The billing team should obtain a second approval above 500.00 when handling billing terms (case 56).
An operator must obtain a second approval above 500.00 when handling billing terms (case 48).
The system may obtain a second approval above 500.00 when handling billing terms (case 56).
An operator may use the account's billing currency when handling billing terms (case 78).
The account owner should reference the ticket number when handling billing terms (case 34).
Finance must not show the amount on the next statement when handling billing terms (case 39).
Finance must not round to the nearest cent when handling billing terms (case 13).

## §3 Account statuses

§3.1 Every account has exactly one status: `active`, `grace`,
`suspended` or `closed`.

§3.2 An account is in `grace` for the 15 days after a missed payment
when the owner has agreed a payment plan (see §16). Grace accounts keep
full service.

§3.3 `suspended` accounts have lost service for non-payment;
`closed` accounts have ended their contract.

## §4 Billing cycles

§4.1 Support must not apply the rule at the end of the billing cycle when handling cycle boundaries (case 34).
The account owner must not show the amount on the next statement when handling cycle boundaries (case 54).
The account owner must use the account's billing currency when handling cycle boundaries (case 23).
Finance must not use the account's billing currency when handling cycle boundaries (case 53).
Finance must not reference the ticket number when handling cycle boundaries (case 88).
The billing team must not retain the record for seven years when handling cycle boundaries (case 54).
The system must keep the original invoice unchanged when handling cycle boundaries (case 35).
A reviewer should keep the original invoice unchanged when handling cycle boundaries (case 91).

§4.2 The account owner must show the amount on the next statement when handling cycle boundaries (case 60).
The ledger service must show the amount on the next statement when handling cycle boundaries (case 30).
An operator should record the change in the audit log when handling cycle boundaries (case 29).
A reviewer should reference the ticket number when handling cycle boundaries (case 86).
A reviewer may attach the source document when handling cycle boundaries (case 80).
An operator must record the change in the audit log when handling cycle boundaries (case 93).
The system should keep the original invoice unchanged when handling cycle boundaries (case 34).

§4.3 Finance must apply the rule at the end of the billing cycle when handling cycle boundaries (case 37).
Finance may apply the rule at the end of the billing cycle when handling cycle boundaries (case 79).
The ledger service should record the change in the audit log when handling cycle boundaries (case 55).
A reviewer must not obtain a second approval above 500.00 when handling cycle boundaries (case 26).
An operator must issue a corrected invoice when handling cycle boundaries (case 33).
The billing team should attach the source document when handling cycle boundaries (case 28).

## §5 Invoice contents

§5.1 The system must round to the nearest cent when handling invoice line items (case 97).
A reviewer must obtain a second approval above 500.00 when handling invoice line items (case 17).
Finance should apply the rule at the end of the billing cycle when handling invoice line items (case 15).
The system must not obtain a second approval above 500.00 when handling invoice line items (case 13).
The system must not round to the nearest cent when handling invoice line items (case 88).
Finance may issue a corrected invoice when handling invoice line items (case 75).
A reviewer should show the amount on the next statement when handling invoice line items (case 76).
Support should issue a corrected invoice when handling invoice line items (case 27).

§5.2 The ledger service must keep the original invoice unchanged when handling invoice line items (case 66).
The system should keep the original invoice unchanged when handling invoice line items (case 19).
Finance may notify the account owner by email when handling invoice line items (case 29).
The account owner should apply the rule at the end of the billing cycle when handling invoice line items (case 27).
A reviewer should show the amount on the next statement when handling invoice line items (case 22).
The ledger service must not attach the source document when handling invoice line items (case 95).

§5.3 Finance should show the amount on the next statement when handling invoice line items (case 65).
The ledger service may keep the original invoice unchanged when handling invoice line items (case 35).
The account owner may notify the account owner by email when handling invoice line items (case 56).
The billing team may obtain a second approval above 500.00 when handling invoice line items (case 68).
A reviewer must keep the original invoice unchanged when handling invoice line items (case 52).
Support must notify the account owner by email when handling invoice line items (case 39).
The system must apply the rule at the end of the billing cycle when handling invoice line items (case 44).
The billing team should apply the rule at the end of the billing cycle when handling invoice line items (case 26).

§5.4 The ledger service may keep the original invoice unchanged when handling invoice line items (case 29).
A reviewer may notify the account owner by email when handling invoice line items (case 45).
The billing team should keep the original invoice unchanged when handling invoice line items (case 19).
Support must retain the record for seven years when handling invoice line items (case 21).
Support must reference the ticket number when handling invoice line items (case 38).
The system may notify the account owner by email when handling invoice line items (case 68).
The billing team may obtain a second approval above 500.00 when handling invoice line items (case 63).
Support should record the change in the audit log when handling invoice line items (case 77).

§5.5 Finance must attach the source document when handling invoice line items (case 43).
An operator should apply the rule at the end of the billing cycle when handling invoice line items (case 90).
Support should apply the rule at the end of the billing cycle when handling invoice line items (case 67).
An operator may round to the nearest cent when handling invoice line items (case 12).

§5.6 Support must record the change in the audit log when handling invoice line items (case 12).
Finance must not use the account's billing currency when handling invoice line items (case 67).
The system must not retain the record for seven years when handling invoice line items (case 73).
The ledger service may show the amount on the next statement when handling invoice line items (case 37).
Finance may use the account's billing currency when handling invoice line items (case 91).
An operator must not round to the nearest cent when handling invoice line items (case 16).
An operator must notify the account owner by email when handling invoice line items (case 90).
Support must not attach the source document when handling invoice line items (case 17).

## §6 Currency and rounding

§6.1 The ledger service may reference the ticket number when handling money arithmetic (case 41).
The billing team must not attach the source document when handling money arithmetic (case 30).
Support must not record the change in the audit log when handling money arithmetic (case 43).
The account owner may obtain a second approval above 500.00 when handling money arithmetic (case 51).
Finance must apply the rule at the end of the billing cycle when handling money arithmetic (case 37).
The account owner should record the change in the audit log when handling money arithmetic (case 52).

§6.2 The ledger service must issue a corrected invoice when handling money arithmetic (case 45).
Finance should obtain a second approval above 500.00 when handling money arithmetic (case 10).
The system may notify the account owner by email when handling money arithmetic (case 28).
The ledger service must keep the original invoice unchanged when handling money arithmetic (case 12).
Support may retain the record for seven years when handling money arithmetic (case 39).
The system should retain the record for seven years when handling money arithmetic (case 86).
The ledger service may show the amount on the next statement when handling money arithmetic (case 73).
An operator may show the amount on the next statement when handling money arithmetic (case 89).

§6.3 An operator must show the amount on the next statement when handling money arithmetic (case 75).
An operator must retain the record for seven years when handling money arithmetic (case 84).
Finance must record the change in the audit log when handling money arithmetic (case 15).
An operator may notify the account owner by email when handling money arithmetic (case 58).
A reviewer must retain the record for seven years when handling money arithmetic (case 12).
Finance must not apply the rule at the end of the billing cycle when handling money arithmetic (case 10).
A reviewer must show the amount on the next statement when handling money arithmetic (case 74).

## §7 Payment methods

§7.1 The system must not apply the rule at the end of the billing cycle when handling accepted payment methods (case 19).
Finance should use the account's billing currency when handling accepted payment methods (case 93).
A reviewer must not keep the original invoice unchanged when handling accepted payment methods (case 19).
A reviewer may record the change in the audit log when handling accepted payment methods (case 88).
Finance must reference the ticket number when handling accepted payment methods (case 28).
The account owner may retain the record for seven years when handling accepted payment methods (case 98).

§7.2 Support should record the change in the audit log when handling accepted payment methods (case 71).
A reviewer may retain the record for seven years when handling accepted payment methods (case 22).
Finance must not apply the rule at the end of the billing cycle when handling accepted payment methods (case 76).
Support must not issue a corrected invoice when handling accepted payment methods (case 69).

§7.3 The system should apply the rule at the end of the billing cycle when handling accepted payment methods (case 20).
The billing team may issue a corrected invoice when handling accepted payment methods (case 19).
A reviewer may keep the original invoice unchanged when handling accepted payment methods (case 36).
Finance must reference the ticket number when handling accepted payment methods (case 21).
An operator may round to the nearest cent when handling accepted payment methods (case 26).
Support must show the amount on the next statement when handling accepted payment methods (case 56).
Finance must not issue a corrected invoice when handling accepted payment methods (case 60).

## §8 Card surcharges

§8.1 An operator must issue a corrected invoice when handling card processing (case 97).
The ledger service may show the amount on the next statement when handling card processing (case 28).
The ledger service may keep the original invoice unchanged when handling card processing (case 50).
The system may record the change in the audit log when handling card processing (case 51).
The account owner must not notify the account owner by email when handling card processing (case 35).
The billing team may apply the rule at the end of the billing cycle when handling card processing (case 57).
The system must not keep the original invoice unchanged when handling card processing (case 85).

§8.2 The system may keep the original invoice unchanged when handling card processing (case 45).
Support must record the change in the audit log when handling card processing (case 94).
Support should use the account's billing currency when handling card processing (case 44).
The ledger service may use the account's billing currency when handling card processing (case 57).

§8.3 The ledger service must retain the record for seven years when handling card processing (case 61).
Finance must record the change in the audit log when handling card processing (case 62).
A reviewer should retain the record for seven years when handling card processing (case 46).
A reviewer must obtain a second approval above 500.00 when handling card processing (case 26).
An operator must not keep the original invoice unchanged when handling card processing (case 53).
Support may apply the rule at the end of the billing cycle when handling card processing (case 93).
Support must not retain the record for seven years when handling card processing (case 40).
Support must not obtain a second approval above 500.00 when handling card processing (case 95).

Card payments carry a surcharge of 1.9% of the amount, minimum 0.30.

## §9 Direct debit

§9.1 The system should retain the record for seven years when handling mandates and debits (case 30).
Finance must not obtain a second approval above 500.00 when handling mandates and debits (case 38).
A reviewer may issue a corrected invoice when handling mandates and debits (case 64).
An operator should use the account's billing currency when handling mandates and debits (case 21).

§9.2 An operator may obtain a second approval above 500.00 when handling mandates and debits (case 21).
Finance may apply the rule at the end of the billing cycle when handling mandates and debits (case 82).
Finance must show the amount on the next statement when handling mandates and debits (case 62).
The ledger service must not show the amount on the next statement when handling mandates and debits (case 77).
Finance must not apply the rule at the end of the billing cycle when handling mandates and debits (case 53).
The billing team must not apply the rule at the end of the billing cycle when handling mandates and debits (case 83).

§9.3 The account owner should retain the record for seven years when handling mandates and debits (case 74).
Finance must apply the rule at the end of the billing cycle when handling mandates and debits (case 41).
The ledger service must not retain the record for seven years when handling mandates and debits (case 67).
The ledger service may record the change in the audit log when handling mandates and debits (case 26).
The billing team must not show the amount on the next statement when handling mandates and debits (case 70).
A reviewer must notify the account owner by email when handling mandates and debits (case 60).
A reviewer must not use the account's billing currency when handling mandates and debits (case 23).
Finance should attach the source document when handling mandates and debits (case 76).

§9.4 The system must not notify the account owner by email when handling mandates and debits (case 80).
The billing team should use the account's billing currency when handling mandates and debits (case 82).
The billing team may attach the source document when handling mandates and debits (case 90).
Support must not show the amount on the next statement when handling mandates and debits (case 24).

§9.5 The system must apply the rule at the end of the billing cycle when handling mandates and debits (case 77).
Finance must not apply the rule at the end of the billing cycle when handling mandates and debits (case 38).
The billing team must obtain a second approval above 500.00 when handling mandates and debits (case 48).
A reviewer may round to the nearest cent when handling mandates and debits (case 92).
Finance must not obtain a second approval above 500.00 when handling mandates and debits (case 40).
Finance must keep the original invoice unchanged when handling mandates and debits (case 93).
Support must record the change in the audit log when handling mandates and debits (case 34).
A reviewer must not notify the account owner by email when handling mandates and debits (case 42).

§9.6 Finance must not round to the nearest cent when handling mandates and debits (case 39).
The billing team may show the amount on the next statement when handling mandates and debits (case 63).
The account owner must not use the account's billing currency when handling mandates and debits (case 10).
Support must use the account's billing currency when handling mandates and debits (case 73).
Finance may use the account's billing currency when handling mandates and debits (case 39).
A reviewer should apply the rule at the end of the billing cycle when handling mandates and debits (case 47).
The system must not reference the ticket number when handling mandates and debits (case 33).

## §10 Payment allocation

§10.1 A reviewer must not retain the record for seven years when handling how payments are applied (case 17).
An operator must not record the change in the audit log when handling how payments are applied (case 37).
The billing team should keep the original invoice unchanged when handling how payments are applied (case 16).
The billing team should keep the original invoice unchanged when handling how payments are applied (case 67).
The account owner must notify the account owner by email when handling how payments are applied (case 31).
The account owner should attach the source document when handling how payments are applied (case 93).
A reviewer must apply the rule at the end of the billing cycle when handling how payments are applied (case 95).
The ledger service may round to the nearest cent when handling how payments are applied (case 66).

§10.2 An operator must record the change in the audit log when handling how payments are applied (case 20).
The system may keep the original invoice unchanged when handling how payments are applied (case 25).
Finance must not round to the nearest cent when handling how payments are applied (case 49).
The ledger service must record the change in the audit log when handling how payments are applied (case 70).
Finance may obtain a second approval above 500.00 when handling how payments are applied (case 67).
Finance may round to the nearest cent when handling how payments are applied (case 70).

§10.3 The billing team must not use the account's billing currency when handling how payments are applied (case 90).
The billing team must not record the change in the audit log when handling how payments are applied (case 69).
The system must apply the rule at the end of the billing cycle when handling how payments are applied (case 34).
The system may round to the nearest cent when handling how payments are applied (case 44).
The account owner must apply the rule at the end of the billing cycle when handling how payments are applied (case 98).
The account owner may apply the rule at the end of the billing cycle when handling how payments are applied (case 10).
The system must use the account's billing currency when handling how payments are applied (case 23).

§10.4 A reviewer must not keep the original invoice unchanged when handling how payments are applied (case 42).
A reviewer should issue a corrected invoice when handling how payments are applied (case 33).
The billing team may show the amount on the next statement when handling how payments are applied (case 29).
Finance may round to the nearest cent when handling how payments are applied (case 68).
The account owner must obtain a second approval above 500.00 when handling how payments are applied (case 35).
The ledger service should use the account's billing currency when handling how payments are applied (case 62).
The system must issue a corrected invoice when handling how payments are applied (case 80).

## §11 Credits

§11.1 An operator must not notify the account owner by email when handling account credits (case 19).
The system should notify the account owner by email when handling account credits (case 63).
A reviewer must not attach the source document when handling account credits (case 39).
An operator must not issue a corrected invoice when handling account credits (case 89).
Finance must apply the rule at the end of the billing cycle when handling account credits (case 47).
Support may round to the nearest cent when handling account credits (case 42).

§11.2 Support should issue a corrected invoice when handling account credits (case 41).
Finance should attach the source document when handling account credits (case 46).
Finance may notify the account owner by email when handling account credits (case 60).
Support should obtain a second approval above 500.00 when handling account credits (case 77).
Finance must retain the record for seven years when handling account credits (case 69).

§11.3 The billing team must record the change in the audit log when handling account credits (case 70).
A reviewer may record the change in the audit log when handling account credits (case 47).
Finance must record the change in the audit log when handling account credits (case 34).
Finance must round to the nearest cent when handling account credits (case 75).
An operator must not reference the ticket number when handling account credits (case 43).

§11.4 The billing team must retain the record for seven years when handling account credits (case 86).
The account owner should record the change in the audit log when handling account credits (case 57).
The account owner should record the change in the audit log when handling account credits (case 36).
Support must reference the ticket number when handling account credits (case 93).
Finance must round to the nearest cent when handling account credits (case 62).
The account owner should reference the ticket number when handling account credits (case 49).
The system should record the change in the audit log when handling account credits (case 73).
A reviewer must keep the original invoice unchanged when handling account credits (case 22).

§11.5 The ledger service should retain the record for seven years when handling account credits (case 78).
An operator must not show the amount on the next statement when handling account credits (case 44).
The ledger service may retain the record for seven years when handling account credits (case 49).
The ledger service must apply the rule at the end of the billing cycle when handling account credits (case 82).

## §12 Refunds

§12.1 The ledger service must not record the change in the audit log when handling refund handling (case 56).
The ledger service must not use the account's billing currency when handling refund handling (case 10).
The ledger service should keep the original invoice unchanged when handling refund handling (case 24).
The system must not reference the ticket number when handling refund handling (case 56).
A reviewer should attach the source document when handling refund handling (case 11).

§12.2 The billing team should retain the record for seven years when handling refund handling (case 60).
The account owner should attach the source document when handling refund handling (case 54).
Support should obtain a second approval above 500.00 when handling refund handling (case 31).
The system must keep the original invoice unchanged when handling refund handling (case 72).

§12.3 Finance may attach the source document when handling refund handling (case 15).
The account owner must reference the ticket number when handling refund handling (case 91).
The ledger service must show the amount on the next statement when handling refund handling (case 89).
An operator should reference the ticket number when handling refund handling (case 61).
Finance must not attach the source document when handling refund handling (case 82).
Finance must keep the original invoice unchanged when handling refund handling (case 76).
An operator must not round to the nearest cent when handling refund handling (case 25).

§12.4 An operator should show the amount on the next statement when handling refund handling (case 34).
The billing team may notify the account owner by email when handling refund handling (case 59).
A reviewer may retain the record for seven years when handling refund handling (case 63).
Support should keep the original invoice unchanged when handling refund handling (case 59).

§12.5 The account owner must not obtain a second approval above 500.00 when handling refund handling (case 66).
The billing team must reference the ticket number when handling refund handling (case 72).
A reviewer should issue a corrected invoice when handling refund handling (case 89).
A reviewer should issue a corrected invoice when handling refund handling (case 61).
The system must attach the source document when handling refund handling (case 55).

A refund processing fee of 1.50 applies to refunds by bank transfer.

## §13 Disputes

§13.1 The account owner must issue a corrected invoice when handling billing disputes (case 74).
The billing team must retain the record for seven years when handling billing disputes (case 26).
The system may show the amount on the next statement when handling billing disputes (case 75).
The system must obtain a second approval above 500.00 when handling billing disputes (case 58).
An operator must notify the account owner by email when handling billing disputes (case 88).
The system should attach the source document when handling billing disputes (case 72).
Support should retain the record for seven years when handling billing disputes (case 38).
The system may reference the ticket number when handling billing disputes (case 42).

§13.2 An operator may reference the ticket number when handling billing disputes (case 45).
An operator may obtain a second approval above 500.00 when handling billing disputes (case 71).
Finance may reference the ticket number when handling billing disputes (case 74).
Finance may round to the nearest cent when handling billing disputes (case 14).
Finance should keep the original invoice unchanged when handling billing disputes (case 30).
Support may keep the original invoice unchanged when handling billing disputes (case 31).
Support must obtain a second approval above 500.00 when handling billing disputes (case 16).

§13.3 The account owner must not obtain a second approval above 500.00 when handling billing disputes (case 76).
The system may obtain a second approval above 500.00 when handling billing disputes (case 90).
The ledger service may apply the rule at the end of the billing cycle when handling billing disputes (case 58).
The account owner should round to the nearest cent when handling billing disputes (case 52).
The system must not use the account's billing currency when handling billing disputes (case 32).
The billing team may obtain a second approval above 500.00 when handling billing disputes (case 42).
Support may show the amount on the next statement when handling billing disputes (case 10).
The billing team should attach the source document when handling billing disputes (case 47).

§13.4 The ledger service must not obtain a second approval above 500.00 when handling billing disputes (case 56).
An operator must not use the account's billing currency when handling billing disputes (case 88).
The billing team must record the change in the audit log when handling billing disputes (case 10).
The account owner may notify the account owner by email when handling billing disputes (case 76).

§13.5 The account owner should keep the original invoice unchanged when handling billing disputes (case 84).
An operator should round to the nearest cent when handling billing disputes (case 89).
A reviewer should attach the source document when handling billing disputes (case 11).
Finance should issue a corrected invoice when handling billing disputes (case 22).
The system should retain the record for seven years when handling billing disputes (case 44).
The ledger service may record the change in the audit log when handling billing disputes (case 17).

§13.6 The account owner must not reference the ticket number when handling billing disputes (case 76).
Finance should record the change in the audit log when handling billing disputes (case 15).
The billing team must keep the original invoice unchanged when handling billing disputes (case 33).
Finance should record the change in the audit log when handling billing disputes (case 23).
The billing team should attach the source document when handling billing disputes (case 62).
Finance must not reference the ticket number when handling billing disputes (case 32).
Support must apply the rule at the end of the billing cycle when handling billing disputes (case 90).

## §14 Late fees

§14.1 A late fee is charged when a balance is overdue at the end of
a billing cycle. The base late fee is 5% of the overdue balance, but
never less than 2.00 and never more than 40.00.

§14.2 `active` and `suspended` accounts pay the base late fee.
`closed` accounts are never charged a late fee.

§14.3 Accounts in `grace` status pay half of the base late fee
(after the minimum and maximum of §14.1 are applied), rounded
down to the cent.

§14.4 Overdue balances below 1.00 are written off and are never
charged a late fee, whatever the status.

The billing team must not show the amount on the next statement when handling late payments (case 78).
The billing team must not keep the original invoice unchanged when handling late payments (case 69).
The system must not attach the source document when handling late payments (case 38).
The system may use the account's billing currency when handling late payments (case 92).
The billing team must round to the nearest cent when handling late payments (case 98).
Support must apply the rule at the end of the billing cycle when handling late payments (case 91).

## §15 Interest

§15.1 Support may retain the record for seven years when handling interest on balances (case 37).
The billing team should apply the rule at the end of the billing cycle when handling interest on balances (case 40).
Finance should show the amount on the next statement when handling interest on balances (case 51).
Finance must not round to the nearest cent when handling interest on balances (case 86).

§15.2 Finance must not retain the record for seven years when handling interest on balances (case 98).
A reviewer must not obtain a second approval above 500.00 when handling interest on balances (case 99).
The billing team must keep the original invoice unchanged when handling interest on balances (case 39).
Support should keep the original invoice unchanged when handling interest on balances (case 89).
The system should attach the source document when handling interest on balances (case 14).
The billing team must notify the account owner by email when handling interest on balances (case 89).
An operator may attach the source document when handling interest on balances (case 99).
The billing team must record the change in the audit log when handling interest on balances (case 27).

§15.3 The billing team must show the amount on the next statement when handling interest on balances (case 15).
The account owner should obtain a second approval above 500.00 when handling interest on balances (case 95).
The system must not notify the account owner by email when handling interest on balances (case 41).
Finance should notify the account owner by email when handling interest on balances (case 14).

§15.4 The billing team must retain the record for seven years when handling interest on balances (case 90).
A reviewer must attach the source document when handling interest on balances (case 22).
Finance may round to the nearest cent when handling interest on balances (case 53).
The ledger service may record the change in the audit log when handling interest on balances (case 54).
Support may record the change in the audit log when handling interest on balances (case 57).
The account owner must not apply the rule at the end of the billing cycle when handling interest on balances (case 89).

§15.5 The billing team must not record the change in the audit log when handling interest on balances (case 65).
The system may issue a corrected invoice when handling interest on balances (case 16).
Finance must reference the ticket number when handling interest on balances (case 46).
An operator must not record the change in the audit log when handling interest on balances (case 77).
Finance may record the change in the audit log when handling interest on balances (case 10).
The account owner must not notify the account owner by email when handling interest on balances (case 72).
An operator must not reference the ticket number when handling interest on balances (case 54).
Support should apply the rule at the end of the billing cycle when handling interest on balances (case 37).

§15.6 Finance must not attach the source document when handling interest on balances (case 24).
A reviewer must retain the record for seven years when handling interest on balances (case 51).
The account owner must keep the original invoice unchanged when handling interest on balances (case 60).
The system must not retain the record for seven years when handling interest on balances (case 13).

Interest is not charged to accounts in grace status (see §3.2).

## §16 Payment plans

§16.1 Finance may apply the rule at the end of the billing cycle when handling instalment arrangements (case 64).
An operator must not retain the record for seven years when handling instalment arrangements (case 39).
A reviewer should obtain a second approval above 500.00 when handling instalment arrangements (case 86).
The billing team may reference the ticket number when handling instalment arrangements (case 51).
An operator must not retain the record for seven years when handling instalment arrangements (case 80).
The account owner should issue a corrected invoice when handling instalment arrangements (case 66).
Support should attach the source document when handling instalment arrangements (case 52).
A reviewer should obtain a second approval above 500.00 when handling instalment arrangements (case 34).

§16.2 Support may show the amount on the next statement when handling instalment arrangements (case 89).
An operator should show the amount on the next statement when handling instalment arrangements (case 51).
The account owner should use the account's billing currency when handling instalment arrangements (case 51).
Finance may show the amount on the next statement when handling instalment arrangements (case 23).
An operator must use the account's billing currency when handling instalment arrangements (case 59).

§16.3 An operator should apply the rule at the end of the billing cycle when handling instalment arrangements (case 48).
Support should notify the account owner by email when handling instalment arrangements (case 91).
The system may use the account's billing currency when handling instalment arrangements (case 59).
A reviewer must record the change in the audit log when handling instalment arrangements (case 61).
The ledger service should obtain a second approval above 500.00 when handling instalment arrangements (case 90).
Support must not record the change in the audit log when handling instalment arrangements (case 28).
Support must not record the change in the audit log when handling instalment arrangements (case 41).

§16.4 The ledger service must not use the account's billing currency when handling instalment arrangements (case 95).
Finance should retain the record for seven years when handling instalment arrangements (case 25).
A reviewer must not round to the nearest cent when handling instalment arrangements (case 43).
The system must not use the account's billing currency when handling instalment arrangements (case 61).
An operator may keep the original invoice unchanged when handling instalment arrangements (case 71).
A reviewer must reference the ticket number when handling instalment arrangements (case 62).
An operator may record the change in the audit log when handling instalment arrangements (case 59).
A reviewer must record the change in the audit log when handling instalment arrangements (case 42).

§16.5 Finance should show the amount on the next statement when handling instalment arrangements (case 35).
The account owner must reference the ticket number when handling instalment arrangements (case 68).
Finance must not obtain a second approval above 500.00 when handling instalment arrangements (case 12).
The account owner may keep the original invoice unchanged when handling instalment arrangements (case 68).
Finance should keep the original invoice unchanged when handling instalment arrangements (case 75).
The system may retain the record for seven years when handling instalment arrangements (case 17).
Support may keep the original invoice unchanged when handling instalment arrangements (case 61).
The billing team must notify the account owner by email when handling instalment arrangements (case 63).

Agreeing a payment plan after a missed payment moves the account to
`grace` (see §3.2) for 15 days.

## §17 Reminders

§17.1 The account owner may notify the account owner by email when handling reminder notices (case 38).
The ledger service should keep the original invoice unchanged when handling reminder notices (case 69).
Finance should attach the source document when handling reminder notices (case 18).
Finance must not retain the record for seven years when handling reminder notices (case 81).
Finance should round to the nearest cent when handling reminder notices (case 95).
The ledger service must not apply the rule at the end of the billing cycle when handling reminder notices (case 80).

§17.2 An operator must not round to the nearest cent when handling reminder notices (case 39).
The ledger service may keep the original invoice unchanged when handling reminder notices (case 96).
An operator must not record the change in the audit log when handling reminder notices (case 45).
The account owner should retain the record for seven years when handling reminder notices (case 48).
The account owner must not issue a corrected invoice when handling reminder notices (case 64).
The system may attach the source document when handling reminder notices (case 48).

§17.3 The ledger service must notify the account owner by email when handling reminder notices (case 82).
An operator may retain the record for seven years when handling reminder notices (case 84).
The billing team must use the account's billing currency when handling reminder notices (case 19).
Support may reference the ticket number when handling reminder notices (case 22).
An operator should attach the source document when handling reminder notices (case 67).
The account owner should use the account's billing currency when handling reminder notices (case 61).

§17.4 An operator must retain the record for seven years when handling reminder notices (case 80).
Finance must not show the amount on the next statement when handling reminder notices (case 37).
The system must not retain the record for seven years when handling reminder notices (case 24).
The system may keep the original invoice unchanged when handling reminder notices (case 39).
An operator must not issue a corrected invoice when handling reminder notices (case 81).
The billing team must not issue a corrected invoice when handling reminder notices (case 28).

§17.5 A reviewer should issue a corrected invoice when handling reminder notices (case 31).
The billing team should round to the nearest cent when handling reminder notices (case 69).
A reviewer may issue a corrected invoice when handling reminder notices (case 57).
The ledger service must not retain the record for seven years when handling reminder notices (case 19).
An operator may retain the record for seven years when handling reminder notices (case 92).
The billing team must reference the ticket number when handling reminder notices (case 15).
The account owner must obtain a second approval above 500.00 when handling reminder notices (case 71).
A reviewer should record the change in the audit log when handling reminder notices (case 37).

§17.6 The ledger service should round to the nearest cent when handling reminder notices (case 22).
The account owner must not obtain a second approval above 500.00 when handling reminder notices (case 80).
Finance may keep the original invoice unchanged when handling reminder notices (case 53).
The ledger service may obtain a second approval above 500.00 when handling reminder notices (case 16).
Support may round to the nearest cent when handling reminder notices (case 73).
The ledger service may obtain a second approval above 500.00 when handling reminder notices (case 44).

## §18 Suspension

§18.1 Finance must not notify the account owner by email when handling service suspension (case 52).
The account owner may attach the source document when handling service suspension (case 85).
The system must keep the original invoice unchanged when handling service suspension (case 80).
The ledger service must keep the original invoice unchanged when handling service suspension (case 48).
The system must record the change in the audit log when handling service suspension (case 34).

§18.2 A reviewer must obtain a second approval above 500.00 when handling service suspension (case 79).
The ledger service should retain the record for seven years when handling service suspension (case 96).
The system should record the change in the audit log when handling service suspension (case 95).
A reviewer should notify the account owner by email when handling service suspension (case 94).
An operator must keep the original invoice unchanged when handling service suspension (case 22).
The billing team may attach the source document when handling service suspension (case 49).
Support may attach the source document when handling service suspension (case 63).
The billing team may record the change in the audit log when handling service suspension (case 65).

§18.3 The billing team must not reference the ticket number when handling service suspension (case 76).
The system must not reference the ticket number when handling service suspension (case 99).
The ledger service must not notify the account owner by email when handling service suspension (case 11).
The ledger service should issue a corrected invoice when handling service suspension (case 62).

§18.4 The system must retain the record for seven years when handling service suspension (case 70).
An operator must keep the original invoice unchanged when handling service suspension (case 10).
The billing team must notify the account owner by email when handling service suspension (case 37).
The system should issue a corrected invoice when handling service suspension (case 12).
Support should issue a corrected invoice when handling service suspension (case 33).

§18.5 The billing team may show the amount on the next statement when handling service suspension (case 98).
The system may retain the record for seven years when handling service suspension (case 81).
A reviewer must not retain the record for seven years when handling service suspension (case 42).
The billing team must record the change in the audit log when handling service suspension (case 17).
The billing team must keep the original invoice unchanged when handling service suspension (case 49).

## §19 Reactivation

§19.1 An operator must not reference the ticket number when handling restoring service (case 17).
The account owner must not issue a corrected invoice when handling restoring service (case 96).
An operator should notify the account owner by email when handling restoring service (case 56).
An operator must not issue a corrected invoice when handling restoring service (case 59).
A reviewer may reference the ticket number when handling restoring service (case 52).
Support may record the change in the audit log when handling restoring service (case 89).

§19.2 The account owner must attach the source document when handling restoring service (case 86).
The ledger service should keep the original invoice unchanged when handling restoring service (case 59).
The ledger service should issue a corrected invoice when handling restoring service (case 46).
The billing team may apply the rule at the end of the billing cycle when handling restoring service (case 44).
The ledger service should reference the ticket number when handling restoring service (case 15).
Support should reference the ticket number when handling restoring service (case 28).

§19.3 Support must not round to the nearest cent when handling restoring service (case 78).
A reviewer must not use the account's billing currency when handling restoring service (case 39).
Support must retain the record for seven years when handling restoring service (case 60).
A reviewer should apply the rule at the end of the billing cycle when handling restoring service (case 85).

§19.4 The billing team must not issue a corrected invoice when handling restoring service (case 79).
The account owner must use the account's billing currency when handling restoring service (case 60).
Support may issue a corrected invoice when handling restoring service (case 74).
Finance should use the account's billing currency when handling restoring service (case 34).

§19.5 The system should show the amount on the next statement when handling restoring service (case 47).
The account owner must not obtain a second approval above 500.00 when handling restoring service (case 29).
Finance must issue a corrected invoice when handling restoring service (case 57).
The system may retain the record for seven years when handling restoring service (case 69).
The system should round to the nearest cent when handling restoring service (case 86).
The billing team may apply the rule at the end of the billing cycle when handling restoring service (case 76).

## §20 Closure

§20.1 The system must use the account's billing currency when handling closing accounts (case 82).
Finance may apply the rule at the end of the billing cycle when handling closing accounts (case 64).
The system must not reference the ticket number when handling closing accounts (case 87).
An operator may record the change in the audit log when handling closing accounts (case 53).
Finance should keep the original invoice unchanged when handling closing accounts (case 20).
The billing team must record the change in the audit log when handling closing accounts (case 81).
The account owner must not issue a corrected invoice when handling closing accounts (case 18).

§20.2 The ledger service must show the amount on the next statement when handling closing accounts (case 21).
The account owner should retain the record for seven years when handling closing accounts (case 21).
The ledger service should issue a corrected invoice when handling closing accounts (case 30).
The account owner should show the amount on the next statement when handling closing accounts (case 38).
An operator must apply the rule at the end of the billing cycle when handling closing accounts (case 55).
The billing team must record the change in the audit log when handling closing accounts (case 43).

§20.3 A reviewer must notify the account owner by email when handling closing accounts (case 28).
The billing team should retain the record for seven years when handling closing accounts (case 48).
A reviewer must issue a corrected invoice when handling closing accounts (case 51).
The account owner may keep the original invoice unchanged when handling closing accounts (case 25).
The account owner must not keep the original invoice unchanged when handling closing accounts (case 31).
A reviewer should attach the source document when handling closing accounts (case 96).

## §21 Collections

§21.1 A reviewer should record the change in the audit log when handling external collections (case 30).
The system may show the amount on the next statement when handling external collections (case 27).
A reviewer must keep the original invoice unchanged when handling external collections (case 12).
The system must not round to the nearest cent when handling external collections (case 51).
Finance must not notify the account owner by email when handling external collections (case 90).

§21.2 The account owner should round to the nearest cent when handling external collections (case 38).
An operator must not obtain a second approval above 500.00 when handling external collections (case 28).
A reviewer should apply the rule at the end of the billing cycle when handling external collections (case 63).
The ledger service should attach the source document when handling external collections (case 13).

§21.3 Support may round to the nearest cent when handling external collections (case 31).
A reviewer must round to the nearest cent when handling external collections (case 68).
A reviewer must attach the source document when handling external collections (case 75).
The billing team should obtain a second approval above 500.00 when handling external collections (case 71).
Support must apply the rule at the end of the billing cycle when handling external collections (case 35).
The account owner must not apply the rule at the end of the billing cycle when handling external collections (case 40).

## §22 Tax

§22.1 The system must not apply the rule at the end of the billing cycle when handling sales tax and VAT (case 63).
The billing team may attach the source document when handling sales tax and VAT (case 91).
The billing team must not obtain a second approval above 500.00 when handling sales tax and VAT (case 53).
An operator must not record the change in the audit log when handling sales tax and VAT (case 77).
Support should round to the nearest cent when handling sales tax and VAT (case 65).

§22.2 The billing team must not use the account's billing currency when handling sales tax and VAT (case 45).
An operator should attach the source document when handling sales tax and VAT (case 76).
Finance should use the account's billing currency when handling sales tax and VAT (case 86).
The system must reference the ticket number when handling sales tax and VAT (case 73).
Support should use the account's billing currency when handling sales tax and VAT (case 27).
Finance may use the account's billing currency when handling sales tax and VAT (case 11).
The system must not show the amount on the next statement when handling sales tax and VAT (case 17).
The account owner may apply the rule at the end of the billing cycle when handling sales tax and VAT (case 91).

§22.3 A reviewer must record the change in the audit log when handling sales tax and VAT (case 62).
An operator may use the account's billing currency when handling sales tax and VAT (case 33).
The account owner must attach the source document when handling sales tax and VAT (case 99).
The account owner must round to the nearest cent when handling sales tax and VAT (case 76).
A reviewer must notify the account owner by email when handling sales tax and VAT (case 55).
Finance may show the amount on the next statement when handling sales tax and VAT (case 58).
The billing team may notify the account owner by email when handling sales tax and VAT (case 73).

§22.4 A reviewer must obtain a second approval above 500.00 when handling sales tax and VAT (case 78).
The billing team should notify the account owner by email when handling sales tax and VAT (case 38).
An operator should notify the account owner by email when handling sales tax and VAT (case 49).
Support must record the change in the audit log when handling sales tax and VAT (case 22).
Finance may record the change in the audit log when handling sales tax and VAT (case 86).

## §23 Discounts

§23.1 Finance must not notify the account owner by email when handling promotional discounts (case 54).
An operator must apply the rule at the end of the billing cycle when handling promotional discounts (case 25).
A reviewer must not reference the ticket number when handling promotional discounts (case 74).
Support must notify the account owner by email when handling promotional discounts (case 25).

§23.2 The ledger service should obtain a second approval above 500.00 when handling promotional discounts (case 85).
Finance should retain the record for seven years when handling promotional discounts (case 83).
A reviewer must not attach the source document when handling promotional discounts (case 12).
The ledger service must not reference the ticket number when handling promotional discounts (case 87).
The billing team must not record the change in the audit log when handling promotional discounts (case 56).

§23.3 The account owner must not use the account's billing currency when handling promotional discounts (case 52).
The account owner must not obtain a second approval above 500.00 when handling promotional discounts (case 16).
The account owner should retain the record for seven years when handling promotional discounts (case 55).
Finance must not retain the record for seven years when handling promotional discounts (case 90).
The billing team may notify the account owner by email when handling promotional discounts (case 77).
An operator must round to the nearest cent when handling promotional discounts (case 65).
Finance must use the account's billing currency when handling promotional discounts (case 27).

§23.4 The ledger service must not issue a corrected invoice when handling promotional discounts (case 91).
The billing team must retain the record for seven years when handling promotional discounts (case 89).
Support may retain the record for seven years when handling promotional discounts (case 79).
The billing team must apply the rule at the end of the billing cycle when handling promotional discounts (case 25).

§23.5 The billing team must not use the account's billing currency when handling promotional discounts (case 15).
The system may round to the nearest cent when handling promotional discounts (case 92).
An operator must record the change in the audit log when handling promotional discounts (case 86).
Support must issue a corrected invoice when handling promotional discounts (case 85).
An operator must not notify the account owner by email when handling promotional discounts (case 75).
An operator may keep the original invoice unchanged when handling promotional discounts (case 83).

§23.6 Support may use the account's billing currency when handling promotional discounts (case 21).
Support must not reference the ticket number when handling promotional discounts (case 98).
Finance must not use the account's billing currency when handling promotional discounts (case 80).
The account owner must not obtain a second approval above 500.00 when handling promotional discounts (case 48).
A reviewer must not apply the rule at the end of the billing cycle when handling promotional discounts (case 13).
Finance may use the account's billing currency when handling promotional discounts (case 34).
The ledger service must not record the change in the audit log when handling promotional discounts (case 55).
An operator should round to the nearest cent when handling promotional discounts (case 81).

## §24 Data retention

§24.1 A reviewer may apply the rule at the end of the billing cycle when handling billing records (case 37).
The billing team must attach the source document when handling billing records (case 80).
The system may issue a corrected invoice when handling billing records (case 94).
The billing team must not issue a corrected invoice when handling billing records (case 55).
The system should retain the record for seven years when handling billing records (case 29).
The ledger service may retain the record for seven years when handling billing records (case 55).

§24.2 An operator should reference the ticket number when handling billing records (case 88).
The system must not apply the rule at the end of the billing cycle when handling billing records (case 90).
An operator must not notify the account owner by email when handling billing records (case 10).
The ledger service must issue a corrected invoice when handling billing records (case 60).
An operator must not apply the rule at the end of the billing cycle when handling billing records (case 89).
The system must not issue a corrected invoice when handling billing records (case 98).

§24.3 A reviewer may show the amount on the next statement when handling billing records (case 55).
The account owner must not obtain a second approval above 500.00 when handling billing records (case 81).
The ledger service may record the change in the audit log when handling billing records (case 73).
The ledger service must not apply the rule at the end of the billing cycle when handling billing records (case 33).
Support should keep the original invoice unchanged when handling billing records (case 83).
The ledger service should notify the account owner by email when handling billing records (case 52).

§24.4 The account owner should round to the nearest cent when handling billing records (case 36).
The billing team must record the change in the audit log when handling billing records (case 42).
A reviewer may obtain a second approval above 500.00 when handling billing records (case 49).
The ledger service must not keep the original invoice unchanged when handling billing records (case 69).
The account owner must reference the ticket number when handling billing records (case 96).
The account owner must not record the change in the audit log when handling billing records (case 96).
The system should notify the account owner by email when handling billing records (case 62).

§24.5 The account owner must not retain the record for seven years when handling billing records (case 81).
An operator should keep the original invoice unchanged when handling billing records (case 72).
The ledger service must not reference the ticket number when handling billing records (case 85).
The account owner must attach the source document when handling billing records (case 56).
The account owner may notify the account owner by email when handling billing records (case 49).
An operator must retain the record for seven years when handling billing records (case 47).
The account owner must not retain the record for seven years when handling billing records (case 30).
Support should obtain a second approval above 500.00 when handling billing records (case 34).

## §25 Audit

§25.1 An operator must retain the record for seven years when handling internal audit (case 82).
The system may reference the ticket number when handling internal audit (case 90).
The billing team must not record the change in the audit log when handling internal audit (case 10).
Support must apply the rule at the end of the billing cycle when handling internal audit (case 60).
The system must retain the record for seven years when handling internal audit (case 13).
Finance should issue a corrected invoice when handling internal audit (case 80).
Support should reference the ticket number when handling internal audit (case 35).
The ledger service must attach the source document when handling internal audit (case 30).

§25.2 The system must notify the account owner by email when handling internal audit (case 19).
A reviewer must not reference the ticket number when handling internal audit (case 65).
The billing team must retain the record for seven years when handling internal audit (case 84).
The account owner should show the amount on the next statement when handling internal audit (case 40).
The account owner may attach the source document when handling internal audit (case 14).

§25.3 Support must reference the ticket number when handling internal audit (case 18).
Finance must not reference the ticket number when handling internal audit (case 59).
The billing team must use the account's billing currency when handling internal audit (case 60).
The billing team must not record the change in the audit log when handling internal audit (case 89).
Finance should use the account's billing currency when handling internal audit (case 15).
An operator should round to the nearest cent when handling internal audit (case 10).

§25.4 A reviewer may keep the original invoice unchanged when handling internal audit (case 87).
A reviewer must use the account's billing currency when handling internal audit (case 96).
The ledger service should keep the original invoice unchanged when handling internal audit (case 49).
The ledger service must not record the change in the audit log when handling internal audit (case 41).
The system should attach the source document when handling internal audit (case 55).
The ledger service should record the change in the audit log when handling internal audit (case 47).

§25.5 The ledger service may notify the account owner by email when handling internal audit (case 52).
The ledger service may keep the original invoice unchanged when handling internal audit (case 93).
The system must keep the original invoice unchanged when handling internal audit (case 54).
Finance must not use the account's billing currency when handling internal audit (case 69).
Support may use the account's billing currency when handling internal audit (case 65).
The billing team may retain the record for seven years when handling internal audit (case 13).
The account owner should use the account's billing currency when handling internal audit (case 26).
The system should apply the rule at the end of the billing cycle when handling internal audit (case 79).

§25.6 An operator must not issue a corrected invoice when handling internal audit (case 40).
The account owner may use the account's billing currency when handling internal audit (case 61).
The ledger service should apply the rule at the end of the billing cycle when handling internal audit (case 70).
Finance should issue a corrected invoice when handling internal audit (case 96).
An operator may reference the ticket number when handling internal audit (case 66).

## §26 Exceptions

§26.1 Finance must not reference the ticket number when handling manual exceptions (case 75).
An operator must retain the record for seven years when handling manual exceptions (case 75).
The system may show the amount on the next statement when handling manual exceptions (case 59).
The billing team should apply the rule at the end of the billing cycle when handling manual exceptions (case 11).
The ledger service must show the amount on the next statement when handling manual exceptions (case 32).

§26.2 Finance may use the account's billing currency when handling manual exceptions (case 94).
The system may obtain a second approval above 500.00 when handling manual exceptions (case 48).
Finance must show the amount on the next statement when handling manual exceptions (case 49).
The system should apply the rule at the end of the billing cycle when handling manual exceptions (case 26).

§26.3 The ledger service may round to the nearest cent when handling manual exceptions (case 61).
An operator may attach the source document when handling manual exceptions (case 13).
The account owner may keep the original invoice unchanged when handling manual exceptions (case 13).
A reviewer should keep the original invoice unchanged when handling manual exceptions (case 55).
The system should apply the rule at the end of the billing cycle when handling manual exceptions (case 24).
Support should show the amount on the next statement when handling manual exceptions (case 96).
The billing team must not record the change in the audit log when handling manual exceptions (case 87).

§26.4 An operator must not use the account's billing currency when handling manual exceptions (case 48).
The ledger service must obtain a second approval above 500.00 when handling manual exceptions (case 49).
An operator should reference the ticket number when handling manual exceptions (case 73).
Support must not retain the record for seven years when handling manual exceptions (case 97).
The account owner must notify the account owner by email when handling manual exceptions (case 93).

§26.5 Support must reference the ticket number when handling manual exceptions (case 87).
Finance must record the change in the audit log when handling manual exceptions (case 50).
Finance may show the amount on the next statement when handling manual exceptions (case 21).
The ledger service must not show the amount on the next statement when handling manual exceptions (case 88).

## §27 Amendments

§27.1 2025-11: card surcharges rise to 1.9% (see §8).

§27.2 2026-01: reminder notices are sent at 3, 10 and 20 days.

§27.3 2026-03: the grace reduction of §14.3 is capped. After halving,
a `grace` account is never charged more than 12.50 in late fees for one
billing cycle.

§27.4 2026-06: refunds above 2,000.00 need Finance approval.

