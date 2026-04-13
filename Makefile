PORT ?= 8066

dev:
	python3 -c 'import run; run.dev()'

.PHONY: server
server:
	python3 viz/server.py $(PORT)
