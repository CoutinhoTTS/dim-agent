import type { SourceDeclaration } from '../../../npm/plugin.js'

// Goal's Settings source, registered by the plugin's row when it starts.
export const settings: SourceDeclaration = {
  "source": "goal",
  "userFile": "goal.json",
  "schema": {
    "type": "object",
    "properties": {
      "maxContinuations": {
        "type": "integer",
        "minimum": 1,
        "maximum": 1000,
        "default": 20
      }
    },
    "additionalProperties": false
  }
}
