# Distribution officielle, iPhone et domaines de Grok Bot

Cette passe identifie l’app iOS officielle et ses identifiants, les associations de domaines publiques, les routes de téléchargement et le flux de connexion Mac. Elle réutilise le paquet **Grok Bot 0.70.2**, sans lancer l’application. Les observations HTTP publiques datent du **17 octobre 2026 UTC** et sont séparées du contenu du paquet. Le [ledger](../evidence/distribution-mobile-discovery.json) conserve les corps publics, leur SHA-256, les commandes de lecture, les extraits et les limites.

La preuve REA réutilisée est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`, dans [rea-bundle.json](../evidence/rea-bundle.json). Le SHA-256 de l’ASAR est `9fe94a66942f8511281f6d7c80a42432d749284c46e1e26f5b6d2f99b87652b0`. Les copies formatées sont des dérivés existants de Prettier 3.6.2; aucun original ou manifeste historique n’est modifié. L’analyse REA générale n’a pas été répétée.

## 1. Le paquet Mac déclare son identité et deux schémas

**DM01 — Métadonnées du paquet et affichage de signature.** `inputs/Grok Bot.app/Contents/Info.plist` déclare `com.anysphere.sand`, version `0.70.2`, exécutable `Grok Bot`, macOS minimum `12.0`, et les schémas URL `grokbot` et `sand`. Le SHA-256 du plist est `a165e9dca2c211dca8974257c45eb27da2a5e7761cc53d84d0a98362a52e4c5a`.

`codesign -dv --verbose=4` affiche `Developer ID Application: Anysphere Incorporated (DCNK4UB866)`, `TeamIdentifier=DCNK4UB866`, un ticket de notarisation agrafé et un exécutable x86_64. `codesign -d --entitlements :-` affiche seulement les autorisations Apple events, JIT, microphone et caméra; **aucun entitlement `com.apple.developer.associated-domains` dans cette signature affichée**. Il s’agit d’une lecture, pas d’une vérification cryptographique de signature. Cela ne décrit pas les entitlements iOS.

## 2. Le bouton mobile mène à une fiche Apple précise

**DM02 — Appel client actif, puis métadonnées publiques actuelles.** Le menu compte construit un item `device-mobile`, dont le callback ouvre `M7e` via `desktop.openExternal`. La constante vaut exactement `https://apps.apple.com/us/app/grok-bot/id6794501026`. Le chemin est établi par le corps du callback, ses imports/exports et l’adaptateur d’ouverture, sans cliquer le bouton : A:28687–28691,81736–81742; U:658–665,24899–24908,25042–25051,25162–25180,638–648; A:80906–80916,5830–5840.

La [fiche App Store de Grok Bot](https://apps.apple.com/us/app/grok-bot/id6794501026) correspond à iPhone/iPad et renvoie vers `https://x.ai/bot`. L’[API publique Apple Lookup](https://itunes.apple.com/lookup?id=6794501026&country=us) donne :

| Champ Apple | Valeur observée |
|---|---|
| `trackId` | `6794501026` |
| `bundleId` | `co.anysphere.sand` |
| `version` | `1.14.0` |
| `minimumOsVersion` | `18.0` |
| `artistName` / `sellerName` | `X Corp.` |
| `sellerUrl` | `https://x.ai/bot` |
| `currentVersionReleaseDate` | `2026-10-06T15:28:37Z` |

Le bundle iOS commence par **`co.`**, le bundle Mac par **`com.`**. L’identité et la distribution sont établies; le framework natif, les classes iOS et les transports internes restent inconnus dans ce volet. Aucun IPA n’est acquis.

## 3. Android possède aussi un canal officiel distinct

**DM03 — Documentation publique officielle.** La [page mobile du produit](https://docs.x.ai/grok-bot/mobile) relie la même fiche Apple et [Google Play, `ai.x.grok.bot`](https://play.google.com/store/apps/details?id=ai.x.grok.bot). Elle indique iOS/iPadOS 18 et Android 9 minimum. La fiche Google présente Grok Bot par SpaceXAI. Le lien est attribué au produit par la documentation, plutôt que par les liens génériques « iOS/Android » du menu global de x.ai, qui peuvent viser Grok principal. Aucun APK n’est téléchargé. Le corps Markdown officiel et les URL exactes sont conservés dans `PUB-MOBILE-DOC`.

## 4. x.ai publie les identités et les routes de partage iOS

**DM04 — Déclaration publique du domaine.** L’[AASA de x.ai](https://x.ai/.well-known/apple-app-site-association), conservée comme `PUB-XAI-AASA`, associe les IDs suivants aux routes Grok Bot :

```text
DCNK4UB866.co.anysphere.sand
DCNK4UB866.co.anysphere.sand.internal
N66CZ3Y3BX.x.ai.grokbot.internal
DCNK4UB866.co.anysphere.sand.dev
N66CZ3Y3BX.co.anysphere.sand
N66CZ3Y3BX.co.anysphere.sand.dev
```

Les composants comprennent `/bot/plugin/?*`, `/bot/?????????????????????` et `/bot/?????????????????????/*`. Les commentaires du document associent les deux derniers motifs aux templates partagés à identifiant de 21 caractères. `/bot/use-cases/*` est explicitement exclu. Un autre détail vise Grok principal, `N66CZ3Y3BX.ai.x.GrokApp`; ce n’est pas l’identité Grok Bot. Les motifs AASA seuls ne garantissent ni validation de l’identifiant par l’app ni ouverture effective sur un appareil. Les variantes internes/dev ne prouvent pas l’existence de builds publiquement téléchargeables.

## 5. L’association Cursor concerne notamment le retour 1Password

**DM05 — Déclaration publique du domaine.** L’[AASA de cursor.com](https://cursor.com/.well-known/apple-app-site-association), `PUB-CURSOR-AASA`, associe `DCNK4UB866.co.anysphere.sand.dev`, `N66CZ3Y3BX.x.ai.grokbot.internal` et `N66CZ3Y3BX.co.anysphere.sand` à `/bot/1password/oauth/callback`. Les mêmes IDs figurent dans `webcredentials.apps`. Le commentaire attribue à Grok Bot mobile la lecture du code et du fragment `integration_key` après ce retour OAuth.

Cette association ne prouve pas un login utilisateur partagé avec Grok ni une connexion iPhone↔Mac. Les détails `/agents` du même fichier concernent les IDs Cursor. La déclaration du domaine n’est pas une inspection des entitlements de l’IPA installé.

## 6. Les liens du Mac et les liens mobiles ont des grammaires distinctes

**DM06 — Corps client statique.** Le parseur Mac configure `grokbot`/`sand`, l’autorité `app`, des routes `/v1/...` et une limite de 2 048 caractères. Pour `bot-template`, le wrapper HTTPS vérifie l’origine `https://cursor.com` avec `/grok-bot/link` ou `/sand/link`. Un constructeur de partage d’agent produit `https://x.ai/bot/agent/<id>`. Sources A:918–957,1088–1107,1124–1149,1169–1178.

Les partages agent, templates et callbacks OAuth sont donc des surfaces à distinguer. Le parseur Mac 0.70.2 et les motifs de l’AASA iOS actuelle ne sont pas interchangeables. Aucun lien profond n’a été ouvert pendant cette passe.

## 7. La connexion Mac passe par Cursor, la liaison Grok est séparée

**DM07 — Chemin client actif et documentation actuelle.** `uAe.login()` appelle `loginCursor`; le handler main délègue à `cursorAccount.login`. Le service choisit `TRe`, qui, hors développement, configure `redirectTarget="sand"` et `supportsSelectedTeamLogin=true`. Il génère 32 octets aléatoires pour le verifier, un challenge SHA-256 et un UUID, puis ouvre par défaut :

```text
https://cursor.com/loginDeepControl
  ?challenge=<challenge>&uuid=<uuid>&mode=login
  &redirectTarget=sand&supportsSelectedTeamLogin=true
```

Le résultat est attendu par `/auth/poll?uuid=<uuid>&verifier=<verifier>` sur le backend configuré, dont le repli est `https://api2.cursor.sh`. Le verifier n’est pas placé dans le lien navigateur. Des surcharges d’environnement/configuration peuvent changer ces origines. Sources A:54489–54495; M:108798–108809,81354–81365,81990–82001,32490–32524,32600–32643,32689–32726,32755–32803,35415–35436,35478–35492,36367–36388. Aucun de ces endpoints de compte n’est appelé par l’enquête.

La [documentation mobile actuelle](https://docs.x.ai/grok-bot/mobile) demande de terminer l’authentification Cursor dans le navigateur. Elle distingue ensuite la liaison d’un compte Grok pour l’accès SuperGrok. Le [site produit actuel](https://x.ai/bot) renvoie aussi « Sign in with your plan » vers `https://grok.com/?product=grok-bot`. Ce lien marketing actuel ne remplace pas le flux contenu dans Mac 0.70.2. L’AASA 1Password n’établit aucun autre lien d’authentification. Les corps du login iOS et de la liaison serveur restent à récupérer.

## 8. Le feed de mise à jour et les téléchargements sont retracés

**DM08 — Appel client actif.** Le core fixe `https://api2.cursor.sh/updates` et construit :

```text
/api/update/<platform>/<product-track>/<current-version>/<machine-id>/stable
stable  -> sand
nightly -> sand-nightly
dogfood -> sand-dogfood
```

Il ajoute `trackChanged=1` si la version courante ne correspond pas au track demandé. Mac emploie une réponse Squirrel `{url,name}`; le mode `iupdate` accepte `{url,version,sha256hash?}`. `fetchUpdate` appelle ce constructeur avec `getMachineId()`, suit les redirections, traite 204 comme absence de mise à jour et valide le JSON. Sources K:30741–30771,30792–30821; M:124882–124900. Aucun machine ID réel n’est lu, aucun feed individuel n’est interrogé.

Les téléchargements manuels passent par une table plateforme/package et `ipe`, utilisée par le chemin « update required ». La table couvre Mac arm64/x64, Windows arm64/x64 et Linux arm64/x64 en deb/rpm/AppImage. Pour Mac x64 : `https://api2.cursor.sh/updates/download/stable/darwin-x64/grok-bot-101a8917d3e40552`. Sources M:570–591,610–668,123689–123704. Ces routes distribuent des clients; elles ne livrent pas le code du moteur serveur.

## 9. Un artefact desktop public plus récent est localisé

**DM09 — Observation HTTP publique actuelle.** Un HEAD sur la route x64 ci-dessus reçoit 302 vers :

```text
https://downloads.cursor.com/grokbot/stable/darwin-x64/0.71.2/Grok_Bot_0.71.2_x64.dmg
```

Un second HEAD répond 200, `Content-Type: application/x-apple-diskimage`, `Content-Length: 156486357`, `Last-Modified: Wed, 14 Oct 2026 17:49:23 GMT`. La chaîne et ses horaires sont conservés dans `PUB-DOWNLOAD-HEAD`. Aucun corps DMG n’a été téléchargé ni extrait. Le nom 0.71.2 et les headers identifient une piste concrète; ils ne prouvent pas le contenu du binaire. L’ETag multipart n’est pas traité comme un SHA-256.

## 10. Ce que les docs disent du mobile et du cloud

**DM10 — Claims documentaires, sans observation d’exécution.** La [documentation mobile](https://docs.x.ai/grok-bot/mobile), mise à jour le 12 octobre, décrit les mêmes bots, conversations, connecteurs et ordinateur cloud entre desktop et mobile; le travail continue après fermeture. Elle cite notifications de résultats/questions/approbations, texte, dictée, voix, pièces jointes et partage iPhone. L’écran distant peut être observé puis repris pour une intervention humaine. Les réglages incluent les plugins; certaines commandes avancées et la démonstration restent sur desktop. Ces indications ne donnent ni protocole APNs, ni politique iOS de fond, ni chemin d’exécution des outils locaux iOS.

L’[overview officiel](https://docs.x.ai/grok-bot/overview) décrit l’ordinateur cloud comme partagé entre les bots d’un compte et persistant lorsque le téléphone ou le laptop est fermé. Il situe l’exécution distante; il ne fournit pas les services qui réalisent la synchronisation. Ces affirmations actuelles complètent les contrats Mac déjà récupérés, sans constituer une capture de synchronisation réelle ni une connexion directe iPhone↔Mac.

## 11. Frontières et checklist

**DM11 — Recherche négative bornée.** Une recherche sur les fichiers texte du client extrait cherche `testflight.apple.com`, `itms-beta://`, l’ID Apple, l’ID Android et `apple-app-site-association`. Les chemins, nombres et premiers contextes sont consignés dans le ledger. Aucun lien TestFlight attribuable au produit n’a été trouvé dans ce corpus ni dans les pages officielles consultées. Cette recherche n’établit pas une absence globale de TestFlight. Aucun IPA/APK, profil utilisateur, secret, endpoint RPC authentifié ou service serveur privé n’est consulté.

| Question | Statut | Résultat ou preuve manquante |
|---|---|---|
| Identité Mac et schémas | answered | DM01, plist et signature affichée |
| Distribution iOS et bundle ID | answered | DM02, appel UI + Apple Lookup |
| Distribution Android et application ID | answered | DM03, liens officiels |
| Domaines associés et routage public | answered | DM04–DM06, AASA et parseurs |
| Login Mac | answered | DM07, corps du client |
| Login iOS et liaison Grok effective | partial | Documentation; code natif et service absents |
| Feed et canaux desktop | answered | DM08–DM09, appelants + HEAD publics |
| Sync, fond, push et outils mobiles | partial | DM10, documentation; transport et exécution inconnus |
| TestFlight | unresolved | Aucun lien attribuable dans la recherche bornée |
| Implémentation native et serveur | unresolved | Pas d’IPA/APK ou de corps moteur acquis ici |

## Sources statiques et reproduction

Les alias renvoient aux fichiers sous `artifacts/formatted/`, avec leur original homonyme sous `artifacts/client/`. Le ledger donne pour chaque extrait le chemin exact, la plage formatée, l’anchor, son SHA-256, les deux hashes de fichier et le node REA. Les champs de plist sont cités comme métadonnées structurées, sans fausse ligne de code.

| Alias | Chemin relatif aux deux racines |
|---|---|
| A | `dist/renderer/assets/index.eager-app-BfVhIMYf.js` |
| U | `dist/renderer/assets/index-D6ReTfRE.js` |
| M | `dist/electron-main/main-app.cjs` |
| K | `dist/electron-main/main-core.cjs` |

Les corps JSON/Markdown publics sont conservés en UTF-8 dans le ledger. Le GET urllib de `x.ai/bot` a reçu 403 : son corps d’erreur est conservé et distingué de la lecture web réussie utilisée pour les liens du site; le corps intégral de cette lecture web n’est pas retenu. `body_sha256` porte sur les octets de chaque corps retenu, et leur décodage est vérifié. Les requêtes ne portent ni cookies ni authentification. Les corps publics actuels, les headers de téléchargement et le code statique du paquet constituent trois classes de preuves séparées.
