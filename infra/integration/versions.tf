terraform {
  required_version = ">= 1.6"

  required_providers {
    outscale = {
      source  = "outscale/outscale"
      version = "1.8.1" # pinned — bump deliberately, review the provider changelog first
    }
  }
}
