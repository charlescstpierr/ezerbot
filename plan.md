# Ronde 2 du ticket 295

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. Ask the user only with a stated, specific reason the control surface cannot reach the target, and only after driving it as far as it goes. If it won't reproduce directly, synthesize the trigger, tighten conditions, or instrument until it fires.
2. Binary-search the cause. Form the candidate hypotheses, then rule them out until one survives. Seed them with `how` over the affected subsystem and the **why** skill for regression history. Each pass, take the split that cuts the most remaining problem space, get runtime evidence, eliminate. When program state is unclear, add instrumentation or logging and read it as the code runs. Don't guess. Drive a long or stubborn hunt with Cursor's `/loop` command. Confirm the surviving *mechanism* with runtime evidence before the step-3 architect/interrogate fan-out.
3. Plan the fix. If it crosses a function boundary, `architect` first. Delegate implementation to a subagent using your configured bug-fix model (default `grok-4.7-xhigh-fast`) with a specific scope.
   Adaptation du brief. Correction minimale par l'owner, sans arena ni revue produit déléguée. Aucune nouvelle abstraction.
4. Verify on the same surface. The original repro now passes. "Inconclusive" or wrong-surface is not a pass. Flag it. Unit tests show branch behavior, not bug absence.
5. Stage the commits so the failing repro lands before the fix in git history. See the **tdd** skill for the failing-test-first cadence when the bug has a cheap local test path. Skip it when the test would be expensive, integration-heavy, or unclear.
6. Run **Opening a PR**.
   PR existante 574. Actualiser son corps et publier la Preuve autorisée. Revue officielle et pret réservés au coordinateur.

## Données et responsabilités

`selectedId` choisit la Page du contexte courant. `detail` contient la réponse affichée. `generation` invalide les lectures du panneau. `reverting` porte une mutation pour une Page. `quoteSerial` protège les choix explicites du composeur. Le serveur et le navigateur communiquent par JSON HTTP sans objet partagé.

`onState` et `openPage` choisissent, `load` lit, `remember` peint, `hideStage` quitte. `revertPage` invalide les lectures et relit après sa mutation. `composer.restore` relit après un refus et peut choisir puis repeindre une Page. Ce dernier chemin doit respecter les mêmes changements de contexte et mutations.

## Checkpoint de débit

Au plus trois heures. Finir d'abord deux rouges produit indépendants. Puis corriger, rejouer la matrice des sites, figer les commits. Les contrôles finaux et le run Mac repartent de la tête finale. Aucun verdict officiel lancé ici.
