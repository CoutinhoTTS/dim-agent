pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Chargement…",
        "No entries." => "Aucune entrée.",
        "No matching entry." => "Aucune entrée correspondante.",
        "Issues" => "Problèmes",
        "sources" => "Sources",
        "skills" => "Skills",
        "enabled" => "activé",
        "shadowed by {name}" => "masqué par {name}",
        "Reloaded." => "Rechargé.",
        "Enabled {name}." => "{name} activé.",
        "Disabled {name}." => "{name} désactivé.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} rechargé : {entries} entrées, {disabled} désactivées."
        }
        "Hooks" => "Hooks",
        "invalid" => "non valide",
        "Saved." => "Enregistré.",
        "Saved {name}." => "{name} enregistré.",
        "No change." => "Aucun changement.",
        "Your text is kept in {path}." => "Votre texte est conservé dans {path}.",
        "The user's own document; always in effect." => {
            "Le document de l'utilisateur ; toujours actif."
        }
        "No .dimcode/hooks.json in this workspace." => {
            "Pas de .dimcode/hooks.json dans cet espace de travail."
        }
        "Owned by plugin {plugin}." => "Appartient au plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "Le document utilisateur désactive tous les hooks (disableAllHooks)."
        }
        "Enter toggle" => "Enter basculer",
        "Ctrl+E edit" => "Ctrl+E modifier",
        "Ctrl+N add" => "Ctrl+N ajouter",
        "Ctrl+R reload" => "Ctrl+R recharger",
        "Ctrl+T run" => "Ctrl+T exécuter",
        "Ctrl+X remove" => "Ctrl+X supprimer",
        "Tab inspect" => "Tab inspecter",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "Supprimer {name} ? Enter confirmer · Esc annuler"
        }
        "↑↓ scroll · Esc back" => "↑↓ défiler · Esc revenir",
        "Added." => "Ajouté.",
        "Removed {name}." => "{name} supprimé.",
        "unavailable" => "indisponible",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "Réinitialiser {name} ? Enter confirmer · Esc annuler"
        }
        "Reset {name} to its catalog entry." => "{name} réinitialisé sur son entrée de catalogue.",
        "Ran {name}." => "{name} exécuté.",
        "The user document changed since it was read; nothing was written." => {
            "Le document utilisateur a changé depuis sa lecture ; rien n'a été écrit."
        }
        "The job changed since it was read; nothing was written." => {
            "La tâche a changé depuis sa lecture ; rien n'a été écrit."
        }
        "The jobs changed since they were read; nothing was written." => {
            "Les tâches ont changé depuis leur lecture ; rien n'a été écrit."
        }
        "Your next edit opens the current document." => {
            "Votre prochaine modification ouvrira le document actuel."
        }
        "Your next add reopens your text." => "Votre prochain ajout rouvrira votre texte.",
        "Scheduler" => "Planificateur",
        "jobs" => "tâches",
        "Added {name}." => "{name} ajouté.",
        "Run {name}: {outcome}." => "{name} exécuté : {outcome}.",
        "Recent runs of {name}" => "Exécutions récentes de {name}",
        "No run yet." => "Aucune exécution pour l'instant.",
        "No next run." => "Aucune exécution à venir.",
        "Next run {instant}." => "Prochaine exécution {instant}.",
        "Last run {outcome} at {instant}." => "Dernière exécution {outcome} à {instant}.",
        "every {minutes} minutes" => "toutes les {minutes} minutes",
        "at {instant}" => "à {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} en {zone}",
        "The job changed since it was read; nothing was removed." => {
            "La tâche a changé depuis sa lecture ; rien n'a été supprimé."
        }
        "The Scheduler has no area document." => "Le planificateur n'a pas de document de domaine.",
        "No scheduled job has that id." => "Aucune tâche planifiée n'a cet id.",
        "This daemon cannot manage {area}." => "Impossible de gérer {area}.",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "Le document supprime {field}, que le propriétaire ne peut pas effacer ; rien n'a été écrit."
        }
        "The document changes no field; nothing was written." => {
            "Le document ne modifie aucun champ ; rien n'a été écrit."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Les réglages des hooks de cet espace de travail ont changé depuis leur lecture ; rien n'a été écrit."
        }
        "Models" => "Modèles",
        "catalog" => "catalogue",
        "overrides" => "surcharges",
        "added" => "ajoutés",
        "Models · {name}" => "Modèles · {name}",
        "undeclared" => "non déclaré",
        "Context window {window} · max output {output}" => {
            "Fenêtre de contexte {window} · sortie max {output}"
        }
        "Endpoint {endpoint}" => "Point de terminaison {endpoint}",
        "Endpoint {endpoint} · override {value}" => {
            "Point de terminaison {endpoint} · surcharge {value}"
        }
        "The provider reports it unavailable." => "Le fournisseur le signale indisponible.",
        "Limits undeclared; edit the model to declare them." => {
            "Limites non déclarées ; modifiez le modèle pour les déclarer."
        }
        "Sign in to the account to use its models." => {
            "Connectez-vous au compte pour utiliser ses modèles."
        }
        "The signed-in account changed; reload to list its models." => {
            "Le compte connecté a changé ; rechargez pour lister ses modèles."
        }
        "The provider has no catalog yet." => "Ce fournisseur n'a pas encore de catalogue.",
        "The provider listed no models." => "Le fournisseur n'a listé aucun modèle.",
        "Listed {count} models." => "{count} modèles listés.",
        "Refreshed the models." => "Modèles actualisés.",
        "Could not list models: {error}. Showing the stored list." => {
            "Impossible de lister les modèles : {error}. La liste enregistrée est affichée."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "Le compte connecté a changé ; le rechargement a effacé les préférences, surcharges et désactivations."
        }
        "The models changed since they were read; nothing was removed." => {
            "Les modèles ont changé depuis leur lecture ; rien n'a été supprimé."
        }
        "The models changed since they were read; nothing was written." => {
            "Les modèles ont changé depuis leur lecture ; rien n'a été écrit."
        }
        "No provider has that id." => "Aucun fournisseur n'a cet id.",
        "No model has that id." => "Aucun modèle n'a cet id.",
        "This model cannot be removed." => "Ce modèle ne peut pas être supprimé.",
        "A Provider's models have no area document." => {
            "Les modèles d'un fournisseur n'ont pas de document de zone."
        }
        "Models have no run operation." => "Les modèles n'ont pas d'opération d'exécution.",
        "The document is not an object; nothing was written." => {
            "Le document n'est pas un objet ; rien n'a été écrit."
        }
        "The document changes no editable field; nothing was written." => {
            "Le document ne modifie aucun champ modifiable ; rien n'a été écrit."
        }
        "The model document names no model." => "Le document de modèle ne nomme aucun modèle.",
        "The provider has no model catalog to write." => {
            "Ce fournisseur n'a pas de catalogue de modèles à écrire."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "Source {source}",
        "Status {status}" => "Statut {status}",
        "Tools {value}" => "Outils {value}",
        "Vision {value}" => "Vision {value}",
        "Reasoning {value}" => "Raisonnement {value}",
        "Modalities {list}" => "Modalités {list}",
        "Description {text}" => "Description {text}",
        "Release date {date}" => "Date de sortie {date}",
        "Pricing {value}" => "Tarif {value}",
        "Rate limit {value}" => "Limite de débit {value}",
        "Matched provider {id}" => "Fournisseur correspondant {id}",
        "Matched model {id}" => "Modèle correspondant {id}",
        "Match strategy {strategy}" => "Stratégie de correspondance {strategy}",
        "Capability overrides {fields}" => "Surcharges de capacités {fields}",
        "yes" => "oui",
        "no" => "non",
        "off" => "désactivé",
        "supported" => "pris en charge",
        _ => return None,
    })
}
