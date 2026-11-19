{
	"patcher": {
		"fileversion": 1,
		"appversion": {
			"major": 8,
			"minor": 6,
			"revision": 5,
			"architecture": "x64",
			"modernui": 1
		},
		"classnamespace": "box",
		"rect": [
			100.0,
			100.0,
			1100.0,
			600.0
		],
		"openinpresentation": 1,
		"default_fontsize": 10.0,
		"default_fontface": 0,
		"default_fontname": "Arial Bold",
		"gridsize": [
			8.0,
			8.0
		],
		"boxanimatetime": 500,
		"description": "FohMixer Claps: plays the hub's clap pattern on Live's beats, from the next bar; MIDI passes through.",
		"devicewidth": 180.0,
		"dependency_cache": [],
		"autosave": 0,
		"boxes": [
			{
				"box": {
					"id": "obj-midiin",
					"maxclass": "newobj",
					"text": "midiin",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						30.0,
						30.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-midiout",
					"maxclass": "newobj",
					"text": "midiout",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						30.0,
						90.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-udpreceive",
					"maxclass": "newobj",
					"text": "udpreceive 39114",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						200.0,
						30.0,
						120.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-route",
					"maxclass": "newobj",
					"text": "route /fohmixer/claps/notes /fohmixer/claps/pattern /fohmixer/claps/silent /fohmixer/claps/on",
					"numinlets": 1,
					"numoutlets": 5,
					"outlettype": [
						"",
						"",
						"",
						"",
						""
					],
					"patching_rect": [
						200.0,
						70.0,
						560.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-notes",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						200.0,
						330.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-lookup",
					"maxclass": "newobj",
					"text": "zl lookup",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						340.0,
						250.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hit",
					"maxclass": "newobj",
					"text": "expr $i1 && $i2 && !$i3",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						340.0,
						290.0,
						160.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-on-change",
					"maxclass": "newobj",
					"text": "change",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"int",
						"int"
					],
					"patching_rect": [
						600.0,
						110.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-on-sel",
					"maxclass": "newobj",
					"text": "sel 1 0",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						""
					],
					"patching_rect": [
						600.0,
						150.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-arm",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						600.0,
						190.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-stop",
					"maxclass": "message",
					"text": "0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						660.0,
						190.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-heard",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						380.0,
						30.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-watchdog",
					"maxclass": "newobj",
					"text": "delay 3000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						380.0,
						110.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-forget",
					"maxclass": "message",
					"text": "set 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						480.0,
						150.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-armed",
					"maxclass": "newobj",
					"text": "i",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						520.0,
						250.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-thisdevice",
					"maxclass": "newobj",
					"text": "live.thisdevice",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"int",
						"int"
					],
					"patching_rect": [
						30.0,
						150.0,
						110.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-start",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						30.0,
						190.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-beats",
					"maxclass": "newobj",
					"text": "metro 4n @quantize 4n @autostart 1 @autostarttime 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						30.0,
						230.0,
						300.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-transport",
					"maxclass": "newobj",
					"text": "transport",
					"numinlets": 2,
					"numoutlets": 9,
					"outlettype": [
						"int",
						"int",
						"float",
						"float",
						"float",
						"",
						"int",
						"float",
						""
					],
					"patching_rect": [
						30.0,
						270.0,
						120.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-beat",
					"maxclass": "newobj",
					"text": "expr ($i1 - 1 + ($f2 >= 240.)) % 4",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						30.0,
						310.0,
						200.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-beat-t",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						30.0,
						350.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-bar",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						120.0,
						390.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-armed-sel",
					"maxclass": "newobj",
					"text": "sel 1",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						520.0,
						290.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-arm-t",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						520.0,
						330.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-disarm",
					"maxclass": "message",
					"text": "0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						580.0,
						370.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-run",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						520.0,
						370.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hit-sel",
					"maxclass": "newobj",
					"text": "sel 1",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						340.0,
						330.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-iter",
					"maxclass": "newobj",
					"text": "iter",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						200.0,
						370.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-makenote",
					"maxclass": "newobj",
					"text": "makenote 100 150",
					"numinlets": 3,
					"numoutlets": 2,
					"outlettype": [
						"float",
						"float"
					],
					"patching_rect": [
						200.0,
						410.0,
						110.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-noteout",
					"maxclass": "newobj",
					"text": "noteout",
					"numinlets": 3,
					"numoutlets": 0,
					"patching_rect": [
						200.0,
						450.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hello-metro",
					"maxclass": "newobj",
					"text": "metro 1000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						760.0,
						230.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hello",
					"maxclass": "newobj",
					"text": "pack /fohmixer/claps 1 v1.55.0 0",
					"numinlets": 4,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						760.0,
						410.0,
						220.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-udpsend",
					"maxclass": "newobj",
					"text": "udpsend 127.0.0.1 39113",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						760.0,
						490.0,
						160.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-title",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						760.0,
						30.0,
						100.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						10.0,
						100.0,
						18.0
					],
					"text": "FohMixer Claps"
				}
			},
			{
				"box": {
					"id": "obj-version",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						870.0,
						30.0,
						60.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						10.0,
						60.0,
						18.0
					],
					"text": "v1.55.0",
					"varname": "version"
				}
			},
			{
				"box": {
					"id": "obj-label-clapping",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						760.0,
						60.0,
						72.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						34.0,
						72.0,
						18.0
					],
					"text": "Clapping"
				}
			},
			{
				"box": {
					"id": "obj-clapping",
					"maxclass": "number",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"",
						"bang"
					],
					"patching_rect": [
						760.0,
						90.0,
						50.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						88.0,
						34.0,
						50.0,
						18.0
					],
					"ignoreclick": 1,
					"parameter_enable": 0
				}
			},
			{
				"box": {
					"id": "obj-send-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						760.0,
						450.0,
						58.0,
						22.0
					]
				}
			}
		],
		"lines": [
			{
				"patchline": {
					"source": [
						"obj-midiin",
						0
					],
					"destination": [
						"obj-midiout",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-udpreceive",
						0
					],
					"destination": [
						"obj-route",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-route",
						0
					],
					"destination": [
						"obj-notes",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-route",
						1
					],
					"destination": [
						"obj-lookup",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-route",
						2
					],
					"destination": [
						"obj-hit",
						2
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-route",
						3
					],
					"destination": [
						"obj-on-change",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-on-change",
						0
					],
					"destination": [
						"obj-on-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-on-sel",
						0
					],
					"destination": [
						"obj-arm",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-on-sel",
						1
					],
					"destination": [
						"obj-stop",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-udpreceive",
						0
					],
					"destination": [
						"obj-heard",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-heard",
						0
					],
					"destination": [
						"obj-watchdog",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-watchdog",
						0
					],
					"destination": [
						"obj-forget",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-watchdog",
						0
					],
					"destination": [
						"obj-stop",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-forget",
						0
					],
					"destination": [
						"obj-on-change",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-arm",
						0
					],
					"destination": [
						"obj-armed",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-stop",
						0
					],
					"destination": [
						"obj-armed",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-stop",
						0
					],
					"destination": [
						"obj-hit",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-thisdevice",
						0
					],
					"destination": [
						"obj-start",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-start",
						0
					],
					"destination": [
						"obj-beats",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-beats",
						0
					],
					"destination": [
						"obj-transport",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-transport",
						2
					],
					"destination": [
						"obj-beat",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-transport",
						1
					],
					"destination": [
						"obj-beat",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-beat",
						0
					],
					"destination": [
						"obj-beat-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-beat-t",
						1
					],
					"destination": [
						"obj-bar",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-bar",
						0
					],
					"destination": [
						"obj-armed",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-armed",
						0
					],
					"destination": [
						"obj-armed-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-armed-sel",
						0
					],
					"destination": [
						"obj-arm-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-arm-t",
						1
					],
					"destination": [
						"obj-disarm",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-arm-t",
						0
					],
					"destination": [
						"obj-run",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-disarm",
						0
					],
					"destination": [
						"obj-armed",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-run",
						0
					],
					"destination": [
						"obj-hit",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-beat-t",
						0
					],
					"destination": [
						"obj-lookup",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-lookup",
						0
					],
					"destination": [
						"obj-hit",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hit",
						0
					],
					"destination": [
						"obj-hit-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hit-sel",
						0
					],
					"destination": [
						"obj-notes",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-notes",
						0
					],
					"destination": [
						"obj-iter",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-iter",
						0
					],
					"destination": [
						"obj-makenote",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-makenote",
						0
					],
					"destination": [
						"obj-noteout",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-makenote",
						1
					],
					"destination": [
						"obj-noteout",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-start",
						0
					],
					"destination": [
						"obj-hello-metro",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hello-metro",
						0
					],
					"destination": [
						"obj-hello",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-run",
						0
					],
					"destination": [
						"obj-hello",
						3
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-stop",
						0
					],
					"destination": [
						"obj-hello",
						3
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-run",
						0
					],
					"destination": [
						"obj-clapping",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-stop",
						0
					],
					"destination": [
						"obj-clapping",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hello",
						0
					],
					"destination": [
						"obj-send-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-send-defer",
						0
					],
					"destination": [
						"obj-udpsend",
						0
					]
				}
			}
		]
	}
}
