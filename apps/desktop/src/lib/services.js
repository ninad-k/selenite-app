export const serviceDefaults = {
  ai: `enabled = false\nbase_url = "https://api.openai.com/v1"\nchat_model = ""\nembedding_model = ""\n# Store your key in this environment variable before launching Selenite.\napi_key_env = "SELENITE_LLM_API_KEY"\nmax_context_chars = 12000\nfolders = []\n`,
  sync: `enabled = false\nprovider = "google_drive"\nendpoint = "https://www.googleapis.com/drive/v3/"\ntoken_env = "SELENITE_SYNC_TOKEN"\nfolder_id = "existing-folder-id"\ninclude_metadata = false\n# For OneDrive use provider = "one_drive" and\n# endpoint = "https://graph.microsoft.com/v1.0/"\n# drive_id = "existing-drive-id"\n`,
  plugins: `enabled = false\n# catalog_url = "https://your-host/catalog.json"\n\n[trusted_catalog_keys]\n# issuer_key_id = "base64-32-byte-public-key"\n\n[grants]\n# plugin_id = ["vault_list", "vault_read"]\n\n[limits]\nfuel = 5000000\nmemory_bytes = 16777216\nmax_module_bytes = 2097152\nmax_message_bytes = 1048576\nmax_host_calls = 64\n`,
  pro: `# Optional restrictions on the issuer and audience of signed entitlements.\n# issuer = "your-deployment"\n# audience = "selenite"\n\n[trusted_keys]\n# key_id = "base64-32-byte-public-key"\n`,
};

export function dirtyConfigurations(drafts) {
  return Object.entries(drafts).filter(([, draft]) => draft.dirty);
}

export function updateConfiguration(draft, source) {
  return { ...draft, source, dirty: source !== draft.original };
}
