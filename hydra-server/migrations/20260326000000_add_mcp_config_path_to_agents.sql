ALTER TABLE metis.agents ADD COLUMN mcp_config_path TEXT DEFAULT NULL;
ALTER TABLE metis.tasks_v2 ADD COLUMN mcp_config JSONB DEFAULT NULL;
