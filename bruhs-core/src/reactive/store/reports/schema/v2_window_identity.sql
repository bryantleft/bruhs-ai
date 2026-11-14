CREATE TABLE weekly_report_jobs_upgrade(id TEXT PRIMARY KEY,preset_id TEXT NOT NULL,window_end INTEGER NOT NULL,payload TEXT NOT NULL);
INSERT INTO weekly_report_jobs_upgrade(rowid,id,preset_id,window_end,payload) SELECT rowid,id,preset_id,window_end,payload FROM weekly_report_jobs;
DROP TABLE weekly_report_jobs;
ALTER TABLE weekly_report_jobs_upgrade RENAME TO weekly_report_jobs;
CREATE TABLE engineering_reports_upgrade(id TEXT PRIMARY KEY,preset_id TEXT NOT NULL,window_end INTEGER NOT NULL,payload TEXT NOT NULL);
INSERT INTO engineering_reports_upgrade(rowid,id,preset_id,window_end,payload) SELECT rowid,id,preset_id,window_end,payload FROM engineering_reports;
DROP TABLE engineering_reports;
ALTER TABLE engineering_reports_upgrade RENAME TO engineering_reports;
INSERT INTO weekly_report_schema(id,version) VALUES(1,2);
