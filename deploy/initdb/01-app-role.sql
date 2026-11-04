-- The app must not run as a superuser: superusers bypass row-level security.
CREATE ROLE cm_app LOGIN PASSWORD 'cm_app_dev_only';
CREATE DATABASE cm OWNER cm_app;
