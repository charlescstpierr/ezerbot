//! Actions qu'aucun bot ne fait, même avec l'accord de l'utilisateur.
//!
//! La liste est une constante du code : ni la base, ni les Paramètres, ni un
//! outil ne la touchent. Un nom d'outil est coupé en mots ; il est interdit
//! quand un verbe d'une règle y figure avec un de ses objets, où que ce soit
//! dans le nom. Un verbe de lecture placé avant le verbe d'action
//! (`list_payments`, `stripe_list_payouts`) rend le nom permis.

use crate::policy::normalize_name;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forbidden {
    Payment,
    Trading,
    Identity,
}

impl Forbidden {
    /// La phrase rendue au bot, qui la répète à l'utilisateur.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Payment => "Je ne peux pas faire ça : les paiements sont interdits aux bots, même avec ton accord. Tu peux le faire toi-même.",
            Self::Trading => "Je ne peux pas faire ça : acheter ou vendre en bourse ou en cryptomonnaie est interdit aux bots, même avec ton accord. Tu peux le faire toi-même.",
            Self::Identity => "Je ne peux pas faire ça : partager un numéro de carte de crédit ou une pièce d’identité est interdit aux bots, même avec ton accord. Tu peux le faire toi-même.",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Payment => "paiement",
            Self::Trading => "bourse ou cryptomonnaie",
            Self::Identity => "carte de crédit ou pièce d’identité",
        }
    }
}

/// `objects` vide : le verbe suffit seul.
struct Rule {
    family: Forbidden,
    verbs: &'static [&'static str],
    objects: &'static [&'static str],
}

const READ: &[&str] = &[
    "list", "get", "retrieve", "search", "read", "fetch", "find", "show", "view", "describe",
    "query", "lookup", "count", "preview", "check", "estimate",
];

// L'ordre compte : `buy_bitcoin` est de la bourse, `buy_item` un paiement.
const RULES: &[Rule] = &[
    Rule {
        family: Forbidden::Trading,
        verbs: &["buy", "sell", "trade", "swap", "short"],
        objects: &[
            "stock",
            "stocks",
            "share",
            "shares",
            "crypto",
            "cryptocurrency",
            "bitcoin",
            "btc",
            "eth",
            "ethereum",
            "coin",
            "coins",
            "etf",
            "option",
            "options",
            "securities",
        ],
    },
    Rule {
        family: Forbidden::Trading,
        verbs: &["trade"],
        objects: &[],
    },
    Rule {
        family: Forbidden::Identity,
        verbs: &[
            "share", "send", "submit", "upload", "provide", "fill", "enter", "attach", "give",
            "transmit", "disclose", "forward", "post",
        ],
        objects: &[
            "credit_card",
            "card_number",
            "passport",
            "driver_license",
            "drivers_license",
            "driving_licence",
            "driving_license",
            "health_card",
            "health_insurance_card",
            "id_card",
            "identity_card",
            "identity_document",
            "id_document",
            "ssn",
            "social_security_number",
            "social_insurance_number",
            "kyc",
        ],
    },
    Rule {
        family: Forbidden::Payment,
        verbs: &["pay", "payout", "purchase", "buy", "subscribe", "refund"],
        objects: &[],
    },
    Rule {
        family: Forbidden::Payment,
        verbs: &[
            "create",
            "make",
            "send",
            "initiate",
            "execute",
            "submit",
            "confirm",
            "capture",
            "process",
            "start",
            "finalize",
            "authorize",
            "transfer",
            "wire",
            "withdraw",
        ],
        objects: &[
            "payment",
            "payments",
            "payout",
            "payouts",
            "transfer",
            "transfers",
            "charge",
            "charges",
            "refund",
            "refunds",
            "subscription",
            "subscriptions",
            "funds",
            "fund",
            "money",
            "checkout",
        ],
    },
    Rule {
        family: Forbidden::Payment,
        verbs: &["place", "submit"],
        objects: &["order", "orders"],
    },
];

pub fn classify(name: &str) -> Option<Forbidden> {
    classify_in("", name)
}

/// Le nom du connecteur ne prête que des objets (« Payments » + `create`) : ses
/// verbes ne rendent jamais interdite une lecture comme `list_payments` d'un
/// connecteur appelé « Trade ».
pub fn classify_in(connector: &str, name: &str) -> Option<Forbidden> {
    let mut words = split(connector);
    let start = words.len();
    words.extend(split(name));
    let tokens: Vec<&str> = words.iter().map(String::as_str).collect();
    RULES
        .iter()
        .find(|rule| {
            tokens.iter().enumerate().skip(start).any(|(at, token)| {
                rule.verbs.contains(token)
                    && !tokens[start..at].iter().any(|t| READ.contains(t))
                    && (rule.objects.is_empty()
                        || rule.objects.iter().any(|object| names(&tokens, object, at)))
            })
        })
        .map(|rule| rule.family)
}

fn split(name: &str) -> Vec<String> {
    normalize_name(name)
        .split(|c: char| c == '_' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Vrai si les mots de `phrase` se suivent dans `tokens` sans recouvrir le verbe en `verb`.
fn names(tokens: &[&str], phrase: &str, verb: usize) -> bool {
    let words: Vec<&str> = phrase.split('_').collect();
    tokens
        .windows(words.len())
        .enumerate()
        .any(|(start, window)| {
            window == words.as_slice() && !(start..start + words.len()).contains(&verb)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_variants_and_server_prefixes_are_recognised() {
        for name in [
            "create_payment",
            "CREATE_PAYMENT",
            "createPayment",
            "CreatePayment",
            "create-payment",
            "create.payment",
            "stripe_create_payment",
            "Stripe.CreatePayment",
            "mcp_stripe_ab12_create_payment",
            "transfer_funds",
            "payment_create",
            "stripe create_charge",
            "pay",
            "purchase_item",
            "capture_payment_intent",
            "submit_order",
        ] {
            assert_eq!(classify(name), Some(Forbidden::Payment), "{name}");
        }
    }

    #[test]
    fn trading_and_identity_are_told_apart_from_payments() {
        assert_eq!(classify("buy_bitcoin"), Some(Forbidden::Trading));
        assert_eq!(classify("sell_stocks"), Some(Forbidden::Trading));
        assert_eq!(classify("alpaca_trade"), Some(Forbidden::Trading));
        assert_eq!(classify("buy_item"), Some(Forbidden::Payment));
        assert_eq!(classify("share_passport"), Some(Forbidden::Identity));
        assert_eq!(classify("sendCreditCard"), Some(Forbidden::Identity));
        assert_eq!(classify("upload_id_card"), Some(Forbidden::Identity));
    }

    #[test]
    fn a_connector_name_lends_objects_but_never_verbs() {
        assert_eq!(classify_in("Trade", "list_payments"), None);
        assert_eq!(classify_in("Buy Me a Coffee", "get_invoice"), None);
        assert_eq!(classify_in("Buy Me a Coffee", "update_profile"), None);
        assert_eq!(classify_in("Payments", "create"), Some(Forbidden::Payment));
        assert_eq!(
            classify_in("Caisse", "create_payment"),
            Some(Forbidden::Payment)
        );
        assert_eq!(classify_in("Crypto", "buy"), Some(Forbidden::Trading));
        assert_eq!(classify_in("", "transfer_funds"), Some(Forbidden::Payment));
    }

    #[test]
    fn reading_and_unrelated_names_stay_allowed() {
        for name in [
            "list_payments",
            "payments_list",
            "get_invoice",
            "retrieve_payment_intent",
            "stripe_list_payouts",
            "get_pay_stub",
            "transfer_issue",
            "git_checkout",
            "cancel_subscription",
            "send_email",
            "list_issues",
            "run_shell",
            "read_file",
            "github_list_issues",
            "gmail_search",
            "get_passport_status",
            "mcp_start_stripe",
            "",
        ] {
            assert_eq!(classify(name), None, "{name}");
        }
    }
}
