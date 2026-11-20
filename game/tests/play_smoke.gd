extends SceneTree

func _initialize() -> void:
	call_deferred("_check_play")

func _check_play() -> void:
	var scene := load("res://main.tscn") as PackedScene
	if scene == null:
		quit(1)
		return
	var game := scene.instantiate()
	root.add_child(game)
	await process_frame
	if not game.has_method("is_playable") or not game.is_playable() or not game.camera.is_current():
		push_error("Die Welt und eine aktive Spielkamera werden benötigt.")
		quit(1)
		return
	game.toggle_pause()
	var before: int = game.core.snapshot().tick
	await create_timer(0.4).timeout
	if int(game.core.snapshot().tick) != before:
		push_error("Die Pausensteuerung muss den Kern anhalten.")
		quit(1)
		return
	game._set_drafting(true)
	game._add_stop(1)
	game._add_stop(4)
	if game.launch_button.disabled or game.draft_marks.is_empty():
		push_error("Zwei gesetzte Haltestellen müssen eine sichtbare Linie ergeben.")
		quit(1)
		return
	game._start_line()
	if not game.draft_stops.is_empty():
		push_error("Der bestätigte Entwurf muss in den Betrieb wechseln.")
		quit(1)
		return
	print("Erkundbare Stadt, Kamera, Pause und Linienstart über das Werkzeug geprüft.")
	quit()
