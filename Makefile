PORT ?= 8066

.PHONY: server
server:
	@echo "serving on http://localhost:$(PORT)/viz/"
	python3 -m http.server $(PORT)
