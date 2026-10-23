# Vérification de la langue de dictée (#187)

Sur Linux, `cargo test --workspace --locked` vérifie :
- le choix anglais puis français enregistré par `POST /api/voice/language`,
  relu dans `GET /api/state` après réouverture ;
- la langue reçue au démarrage et conservée jusqu'au résultat par le backend
  simulé, sans microphone, reconnaissance Apple ni service distant ;
- le passage de la langue mémorisée au pont Speech par le compositeur,
  même quand l'interface reste en français (tests Node exécutés par Cargo).

Le backend simulé fournit un texte déterministe : il prouve le transport de
la locale, pas la qualité de reconnaissance acoustique.

Le test `french_audio_recognized_on_device_after_english` est marqué
« Mac seulement » et ignoré sur Linux. Sur Mac, Cargo lance le véritable moteur
Apple hors ligne sur les fixtures anglaise puis française. Il faut avoir accordé
l'autorisation Speech et installé les données en-US/fr-CA au préalable.
Exécution directe : `bash tools/test-dictation-fixture.sh`.

Le sélecteur des langues installées, le mode voix et les captures du réglage
restent dans #169. Cette issue fournit la persistance et la preuve portable
nécessaires à sa reprise.
