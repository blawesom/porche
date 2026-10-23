# Credentials NEVER live in this repository. They come from the use2
# profile of ~/.osc/config.json (the only permitted account — see
# README.md / Makefile guard) and are exported by make:
#   export OSC_PROFILE=use2
#   export OSC_ACCESS_KEY=$(jq -r .use2.access_key ~/.osc/config.json)
#   export OSC_SECRET_KEY=$(jq -r .use2.secret_key ~/.osc/config.json)
#   export OSC_REGION=$(jq -r .use2.region ~/.osc/config.json)
provider "outscale" {
  region = var.region
}
