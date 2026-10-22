# Formulaires de site privés

La carte affiche l’origine confirmée et indique : « Jamais montré à votre bot. Enregistré de façon sécurisée dans le coffre local. » La soumission conserve une enveloppe `{origin, values}` dans le coffre local, sans remplissage du site. Le bot reçoit uniquement le statut « enregistré dans le coffre local », jamais les valeurs.

La limite de 4096 octets porte sur toute l’enveloppe JSON, origine et noms des champs compris. Une soumission trop volumineuse est refusée avant écriture avec une erreur en français. La carte affiche cette erreur et conserve les valeurs saisies jusqu’à une soumission réussie. Chaque valeur de champ non vide fait aussi partie des valeurs masquées dans les sorties.
