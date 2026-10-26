# Pilote de reproduction

La base produit des rouges canoniques est caaf0734037a4bd3536e291d365b111f8c9ed219. Le fichier pages.js n’avait aucune modification. Le commit 5d237b47 enregistre les trois tests rouges avant la correction produit.

Le premier essai avait deux rouges produit de navigation et un timeout de fixture. Le suivant gardait deux rouges produit mais la saisie ne plaçait pas encore le caret. Ces essais restent dans red-fixture.log et red-fixture2.log et ne prouvent pas la deuxième classe.

Une retenue de GET par ordre d’arrivée pouvait viser le panneau au lieu du composeur. L’essai red-fixture3.log le montre, il n’est pas retenu comme reproduction de reponse-perimee. La version canonique attribue la requête par sa pile au bord fetch, retient la lecture du composeur, puis observe le DOM avec MutationObserver. red.log donne trois assertions produit rouges sans annulation, skip ou todo.

Le premier test de polling ajouté après la correction attendait que la sélection reste active après la fin d’un envoi. app.js redonne explicitement le focus au composeur à cette fin. green-matrix.log garde cet échec de test incorrect. La version corrigée vérifie la sélection pendant le polling, avant le retour de focus, puis vérifie le canevas conservé et le focus du composeur après le refus. Cette correction du test est journalisée par dlog. Aucune des assertions rouges produit initiales n’a été affaiblie.

Le pilote Mac de ronde 2 est dérivé du pilote r1 relu cette session. Son chemin de run est distinct. Il ajoute une retenue de la vraie réponse HTTP de restauration, puis quitte vers un fil vide et revient par le bouton Page avant de libérer cette réponse. Il ne fabrique ni la réponse ni son état serveur. Les courses de refus et de composeur restent couvertes avec le serveur HTTP contrôlé de la suite navigateur.

Les essais de l’inventaire ont aussi gardé une annulation pendant un démarrage Chrome lent, sans appel HTTP du test concerné. Le budget de chaque nouveau cas passe à 60 secondes, comme le test navigateur r1. Aucune assertion n’est retirée. La matrice entière doit passer avant livraison.

Le fichier DevToolsActivePort peut exister avant que sa première ligne soit écrite. pending-red.log conserve un échec de fixture sur le port 0. La boucle attend maintenant une valeur non vide, au lieu de consommer ses 500 essais sans attendre. Il ne s’agit pas d’une relance après assertion produit.

L’inventaire a ajouté deux reproductions de cohérence pendant le retour. pending-red.log montre le bouton Restauration caché avec detail vide. return-title-red.log montre le titre Page b pendant le retour vers Page a. Chaque symptôme a reçu une correction locale avant le contrôle final.

Le parcours Mac entier, Launch et Cleanup compris, s’exécute sous un seul test-gate. Le pilote interne ne reprend pas ce verrou. HOME du lanceur, TMPDIR et profil sont dans r2. CARGO_GATE_DIR et TEST_GATE_DIR sont épinglés aux verrous globaux. La vraie réponse POST de restauration est lue en mémoire et sa livraison au panneau est retardée pour photographier le retour. Son corps et son statut sont copiés sans modification, afin que le délai de capture ne coupe pas le corps HTTP déjà reçu.

Deux premiers parcours Mac n’avaient pas --sphere pages dans leur commande Launch. launch.py active alors EZERBOT_FAKE_SIGNIN, dont la réponse de secours est OK. Le second diagnostic prouve la demande exacte envoyée, le fournisseur prêt, la réponse OK et l’absence de Page. Le troisième parcours ajoute l’option pages et observe la vraie carte write_page. Les deux premiers runs sont nettoyés et conservés dans attempts.json. Ce défaut du pilote ne compte pas comme une reproduction produit.

Le parcours complet final3 retourne vers la Page pendant la restauration et produit 28 captures. Ce changement de fil ferme la liste de commentaires. Un parcours ciblé treated, sur la même tête, complète les captures du passage 21 € disparu et libellé Traité. Aucun fichier produit ne change entre ces parcours.
