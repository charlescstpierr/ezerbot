# Contrat historique de l'analyse Grok Bot

Ce contrat concerne les délégations de la première analyse, avant la création du socle Rust. La reprise du 13 octobre 2026 autorise explicitement sa consolidation. Les scopes actuels sont attribués par le responsable et décrits dans `reports/consolidation-rust.md`. Les preuves brutes et l’application restent immuables.

La cible autorisée est l'application installée `/Applications/Grok Bot.app`, version 0.70.2. Le dossier extrait est `artifacts/client`. Ne modifiez pas l'application, ses comptes, ses sessions, ses secrets, ni les services distants. Aucun contournement d'authentification ou achat.

Le livrable demandé est une analyse et une architecture Rust. N'écrivez pas de code produit. Votre périmètre d'écriture est uniquement le rapport qui vous est assigné. Ne démarrez pas d'analyse REA en doublon. Le responsable exporte et transmet les références REA communes.

Pour chaque conclusion, fournissez le symbole réel, le chemin extrait et des offsets de caractères ou des lignes de la copie formatée. Distinguez observation statique, observation d'exécution, inférence et choix de conception. Un nom de fichier ou une chaîne isolée ne suffit pas pour prouver un comportement. Tracez le point d'entrée, ses appels, les changements d'état, puis le résultat.

Conservez les limites. Les fichiers `.map` sont absents dans l'ASAR initialement inventorié. Les sources extraites sont des bundles compilés, pas les sources TypeScript originales. Les modules natifs présentant une contradiction d'intégrité sont `observed-untrusted`, même quand leur signature Apple est valide.

Retournez un rapport détaillé, les preuves, les inconnues et les obligations de reconstruction propres à votre périmètre. Le responsable réconcilie la matrice commune.
