pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Chargement…",
        "No entries." => "Aucune entrée.",
        "No matching entry." => "Aucune entrée correspondante.",
        "Issues" => "Problèmes",
        "sources" => "Sources",
        "skills" => "Skills",
        "hooks" => "hooks",
        "scheduled jobs" => "tâches planifiées",
        "workflow definitions" => "définitions de flux de travail",
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
        "There is no hooks file there." => "Il n'y a pas de fichier de hooks à cet endroit.",
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
        "A scheduled job's run takes no input document." => {
            "L'exécution d'une tâche planifiée ne prend aucun document d'entrée."
        }
        "This daemon cannot manage {area}." => "Impossible de gérer {area}.",
        "This daemon lists {area} but cannot toggle them." => {
            "Le daemon liste {area}, mais ne peut pas les basculer."
        }
        "This daemon lists {area} but cannot edit them." => {
            "Le daemon liste {area}, mais ne peut pas les modifier."
        }
        "This daemon lists {area} but cannot reload them." => {
            "Le daemon liste {area}, mais ne peut pas les recharger."
        }
        "This daemon lists {area} but cannot add them." => {
            "Le daemon liste {area}, mais ne peut pas les ajouter."
        }
        "This daemon lists {area} but cannot remove them." => {
            "Le daemon liste {area}, mais ne peut pas les supprimer."
        }
        "This daemon lists {area} but cannot run them." => {
            "Le daemon liste {area}, mais ne peut pas les exécuter."
        }
        "This daemon lists {area} but cannot inspect them." => {
            "Le daemon liste {area}, mais ne peut pas afficher leur détail ou leur historique."
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "Le document supprime {field}, que le propriétaire ne peut pas effacer ; rien n'a été écrit."
        }
        "The document changes no field; nothing was written." => {
            "Le document ne modifie aucun champ ; rien n'a été écrit."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Les réglages des hooks de cet espace de travail ont changé depuis leur lecture ; rien n'a été écrit."
        }
        "Workflow" => "Flux de travail",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name} enregistré. Il reste désactivé jusqu'à ce que vous l'activiez."
        }
        "The definitions changed since they were read; nothing was written." => {
            "Les définitions ont changé depuis leur lecture ; rien n'a été écrit."
        }
        "A definition's name is its key; add it to save it under another name." => {
            "Le nom d'une définition est sa clé ; ajoutez-la pour l'enregistrer sous un autre nom."
        }
        "No project definition is named {name}." => {
            "Aucune définition de projet ne s'appelle {name}."
        }
        "No workflow definition is named {name}." => {
            "Aucune définition de flux de travail ne s'appelle {name}."
        }
        "Saved in the user workflow source; always enabled." => {
            "Enregistré dans la source de flux de travail de l'utilisateur ; toujours activé."
        }
        "Starts only after you enable it." => "Ne démarre qu'après que vous l'avez activé.",
        "Defined by package {package}." => "Défini par le paquet {package}.",
        "{name} — {description} (required)" => "{name} — {description} (requis)",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — requis",
        "{name} — default: {default}" => "{name} — par défaut : {default}",
        "and {count} more" => "et {count} de plus",
        "Recent runs" => "Exécutions récentes",
        "A workflow definition is saved as an object with a source and a document." => {
            "Une définition de flux de travail est enregistrée comme un objet avec une source et un document."
        }
        "A definition is saved to the user source or the project source." => {
            "Une définition est enregistrée dans la source utilisateur ou la source du projet."
        }
        "Started {name}: {run_id}." => "{name} démarré : {run_id}.",
        "Parameters are a JSON object of strings." => {
            "Les paramètres sont un objet JSON de chaînes."
        }
        _ => return None,
    })
}
