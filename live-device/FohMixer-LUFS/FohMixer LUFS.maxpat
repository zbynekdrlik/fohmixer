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
			1400.0,
			900.0
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
		"description": "FohMixer LUFS: BS.1770-4 short-term loudness and Voice, sent to the FohMixer hub on 127.0.0.1.",
		"devicewidth": 210.0,
		"boxes": [
			{
				"box": {
					"id": "obj-plugin",
					"maxclass": "newobj",
					"text": "plugin~",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"signal",
						"signal"
					],
					"patching_rect": [
						30.0,
						30.0,
						76.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-silence",
					"maxclass": "newobj",
					"text": "sig~ 0.",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						80.0,
						76.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-plugout",
					"maxclass": "newobj",
					"text": "plugout~",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"signal",
						"signal"
					],
					"patching_rect": [
						30.0,
						120.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-gen",
					"maxclass": "newobj",
					"text": "gen~",
					"numinlets": 2,
					"numoutlets": 4,
					"outlettype": [
						"signal",
						"signal",
						"signal",
						"signal"
					],
					"patching_rect": [
						30.0,
						90.0,
						220.0,
						22.0
					],
					"patcher": {
						"fileversion": 1,
						"appversion": {
							"major": 8,
							"minor": 6,
							"revision": 5,
							"architecture": "x64",
							"modernui": 1
						},
						"classnamespace": "dsp.gen",
						"rect": [
							100.0,
							100.0,
							720.0,
							680.0
						],
						"boxes": [
							{
								"box": {
									"id": "obj-1",
									"maxclass": "codebox",
									"code": "// FohMixer LUFS (#151, spec docs/superpowers/specs/2026-10-26-lufs-design.md §3.3):\n// BS.1770-4 short-term loudness and the gated Voice reading of a stereo input.\n// This file is the readable copy of the code the gen~ in \"FohMixer LUFS.maxpat\"\n// embeds; scripts/check_lufs_device.py keeps the two equal. The device's\n// numbers are checked on the PC against Sonible trueLevel's short-term (§9).\n//\n// in 1, in 2: left, right.\n// out 1, out 2: silence, unused (the patch feeds plugout~ from sig~ 0., §3.2).\n// out 3: Short-term (S), out 4: Voice (V), LUFS in -70..0 rounded to 0.1,\n// updated at the end of each 100 ms sub-block and held in between.\n\n// State between samples. Every History is read once into a local at the\n// top and written once at the bottom, so the code means the same whether a\n// History written in a sample reads back new or old within it.\nHistory rate_seen(0);\nHistory pb0(0), pb1(0), pb2(0), pa1(0), pa2(0), ra1(0), ra2(0);\nHistory block_len(4800);\nHistory gate_sum(0);\n// The filters' states (direct form II transposed), per channel.\nHistory lp1(0), lp2(0), lr1(0), lr2(0);\nHistory rp1(0), rp2(0), rr1(0), rr2(0);\n// The running sub-block; the window's rings (each sub-block's mean square\n// and whether it is voiced, WINDOW_BLOCKS long, starting silent) and the\n// next slot to write.\nHistory filled(0), acc(0), head(0);\nHistory short_term(-70), voice(-70);\nData blocks(30);\nData voiced(30);\n\n// The constants (§3.3).\nSHELF_F0 = 1681.974450955533;\nSHELF_GAIN_DB = 3.999843853973347;\nSHELF_Q = 0.7071752369554196;\nSHELF_VB_EXP = 0.4996667741545416;\nHIGHPASS_F0 = 38.13547087602444;\nHIGHPASS_Q = 0.5003270373238773;\nLUFS_OFFSET = -0.691;\nBLOCKS_PER_SECOND = 10;\nWINDOW_BLOCKS = 30;\nMOMENTARY_BLOCKS = 4;\nVOICE_GATE_LUFS = -40;\nMIN_VOICED = 10;\nFLOOR_LUFS = -70;\nTOP_LUFS = 0;\n\nc_pb0 = pb0; c_pb1 = pb1; c_pb2 = pb2; c_pa1 = pa1; c_pa2 = pa2;\nc_ra1 = ra1; c_ra2 = ra2; c_len = block_len; c_gate = gate_sum;\ns_lp1 = lp1; s_lp2 = lp2; s_lr1 = lr1; s_lr2 = lr2;\ns_rp1 = rp1; s_rp2 = rp2; s_rr1 = rr1; s_rr2 = rr2;\ns_filled = filled; s_acc = acc; s_head = head;\ns_short = short_term; s_voice = voice;\n\ni = 0; k = 0; vh = 0; vb = 0; a0 = 0; kh = 0; a0h = 0;\nif (samplerate != rate_seen) {\n\t// The coefficients and the sub-block length for this rate (the\n\t// analog prototypes, libebur128's form); a new rate starts over.\n\tk = tan(pi * SHELF_F0 / samplerate);\n\tvh = pow(10, SHELF_GAIN_DB / 20);\n\tvb = pow(vh, SHELF_VB_EXP);\n\ta0 = 1 + k / SHELF_Q + k * k;\n\tc_pb0 = (vh + vb * k / SHELF_Q + k * k) / a0;\n\tc_pb1 = 2 * (k * k - vh) / a0;\n\tc_pb2 = (vh - vb * k / SHELF_Q + k * k) / a0;\n\tc_pa1 = 2 * (k * k - 1) / a0;\n\tc_pa2 = (1 - k / SHELF_Q + k * k) / a0;\n\tkh = tan(pi * HIGHPASS_F0 / samplerate);\n\ta0h = 1 + kh / HIGHPASS_Q + kh * kh;\n\tc_ra1 = 2 * (kh * kh - 1) / a0h;\n\tc_ra2 = (1 - kh / HIGHPASS_Q + kh * kh) / a0h;\n\tc_len = floor(samplerate / BLOCKS_PER_SECOND + 0.5);\n\tc_gate = MOMENTARY_BLOCKS * pow(10, (VOICE_GATE_LUFS - LUFS_OFFSET) / 10);\n\ts_lp1 = 0; s_lp2 = 0; s_lr1 = 0; s_lr2 = 0;\n\ts_rp1 = 0; s_rp2 = 0; s_rr1 = 0; s_rr2 = 0;\n\ts_filled = 0; s_acc = 0; s_head = 0;\n\ts_short = FLOOR_LUFS; s_voice = FLOOR_LUFS;\n\tfor (i = 0; i < WINDOW_BLOCKS; i += 1) {\n\t\tpoke(blocks, 0, i);\n\t\tpoke(voiced, 0, i);\n\t}\n}\n\n// K-weighting, left: the shelf, then the RLB high-pass (b = 1, -2, 1).\nxl = in1;\nyl = c_pb0 * xl + s_lp1;\ns_lp1 = c_pb1 * xl - c_pa1 * yl + s_lp2;\ns_lp2 = c_pb2 * xl - c_pa2 * yl;\nzl = yl + s_lr1;\ns_lr1 = -2 * yl - c_ra1 * zl + s_lr2;\ns_lr2 = yl - c_ra2 * zl;\n// Right.\nxr = in2;\nyr = c_pb0 * xr + s_rp1;\ns_rp1 = c_pb1 * xr - c_pa1 * yr + s_rp2;\ns_rp2 = c_pb2 * xr - c_pa2 * yr;\nzr = yr + s_rr1;\ns_rr1 = -2 * yr - c_ra1 * zr + s_rr2;\ns_rr2 = yr - c_ra2 * zr;\n\n// The sub-block: the mean square summed over the two channels.\ns_acc = s_acc + zl * zl + zr * zr;\ns_filled = s_filled + 1;\nms = 0; earlier = 0; total = 0; vsum = 0; vcount = 0; sl = 0; vl = 0;\nif (s_filled >= c_len) {\n\tms = s_acc / c_len;\n\ts_acc = 0;\n\ts_filled = 0;\n\t// Voiced: this sub-block and the three before it reach the gate.\n\tfor (i = 1; i < MOMENTARY_BLOCKS; i += 1) {\n\t\tearlier = earlier + peek(blocks, wrap(s_head - i, 0, WINDOW_BLOCKS));\n\t}\n\tpoke(blocks, ms, s_head);\n\tpoke(voiced, ms + earlier >= c_gate, s_head);\n\ts_head = wrap(s_head + 1, 0, WINDOW_BLOCKS);\n\tfor (i = 0; i < WINDOW_BLOCKS; i += 1) {\n\t\ttotal = total + peek(blocks, i);\n\t\tif (peek(voiced, i) > 0) {\n\t\t\tvsum = vsum + peek(blocks, i);\n\t\t\tvcount = vcount + 1;\n\t\t}\n\t}\n\tsl = clamp(LUFS_OFFSET + 10 * log10(max(total / WINDOW_BLOCKS, 1e-30)), FLOOR_LUFS, TOP_LUFS);\n\tvl = FLOOR_LUFS;\n\tif (vcount >= MIN_VOICED) {\n\t\tvl = clamp(LUFS_OFFSET + 10 * log10(max(vsum / vcount, 1e-30)), FLOOR_LUFS, TOP_LUFS);\n\t}\n\t// Rounded to 0.1 here, so the patch's [change] passes only a new value.\n\ts_short = floor(sl * 10 + 0.5) / 10;\n\ts_voice = floor(vl * 10 + 0.5) / 10;\n}\n\nrate_seen = samplerate;\npb0 = c_pb0; pb1 = c_pb1; pb2 = c_pb2; pa1 = c_pa1; pa2 = c_pa2;\nra1 = c_ra1; ra2 = c_ra2; block_len = c_len; gate_sum = c_gate;\nlp1 = s_lp1; lp2 = s_lp2; lr1 = s_lr1; lr2 = s_lr2;\nrp1 = s_rp1; rp2 = s_rp2; rr1 = s_rr1; rr2 = s_rr2;\nfilled = s_filled; acc = s_acc; head = s_head;\nshort_term = s_short; voice = s_voice;\n\nout1 = 0;\nout2 = 0;\nout3 = s_short;\nout4 = s_voice;\n",
									"fontface": 0,
									"fontname": "<Monaco>",
									"fontsize": 12.0,
									"numinlets": 2,
									"numoutlets": 4,
									"outlettype": [
										"",
										"",
										"",
										""
									],
									"patching_rect": [
										50.0,
										80.0,
										600.0,
										500.0
									]
								}
							},
							{
								"box": {
									"id": "obj-2",
									"maxclass": "newobj",
									"text": "in 1",
									"numinlets": 0,
									"numoutlets": 1,
									"outlettype": [
										""
									],
									"patching_rect": [
										50.0,
										30.0,
										30.0,
										22.0
									]
								}
							},
							{
								"box": {
									"id": "obj-3",
									"maxclass": "newobj",
									"text": "in 2",
									"numinlets": 0,
									"numoutlets": 1,
									"outlettype": [
										""
									],
									"patching_rect": [
										620.0,
										30.0,
										30.0,
										22.0
									]
								}
							},
							{
								"box": {
									"id": "obj-4",
									"maxclass": "newobj",
									"text": "out 1",
									"numinlets": 1,
									"numoutlets": 0,
									"patching_rect": [
										50.0,
										600.0,
										37.0,
										22.0
									]
								}
							},
							{
								"box": {
									"id": "obj-5",
									"maxclass": "newobj",
									"text": "out 2",
									"numinlets": 1,
									"numoutlets": 0,
									"patching_rect": [
										240.0,
										600.0,
										37.0,
										22.0
									]
								}
							},
							{
								"box": {
									"id": "obj-6",
									"maxclass": "newobj",
									"text": "out 3",
									"numinlets": 1,
									"numoutlets": 0,
									"patching_rect": [
										430.0,
										600.0,
										37.0,
										22.0
									]
								}
							},
							{
								"box": {
									"id": "obj-7",
									"maxclass": "newobj",
									"text": "out 4",
									"numinlets": 1,
									"numoutlets": 0,
									"patching_rect": [
										620.0,
										600.0,
										37.0,
										22.0
									]
								}
							}
						],
						"lines": [
							{
								"patchline": {
									"destination": [
										"obj-1",
										0
									],
									"source": [
										"obj-2",
										0
									]
								}
							},
							{
								"patchline": {
									"destination": [
										"obj-1",
										1
									],
									"source": [
										"obj-3",
										0
									]
								}
							},
							{
								"patchline": {
									"destination": [
										"obj-4",
										0
									],
									"source": [
										"obj-1",
										0
									]
								}
							},
							{
								"patchline": {
									"destination": [
										"obj-5",
										0
									],
									"source": [
										"obj-1",
										1
									]
								}
							},
							{
								"patchline": {
									"destination": [
										"obj-6",
										0
									],
									"source": [
										"obj-1",
										2
									]
								}
							},
							{
								"patchline": {
									"destination": [
										"obj-7",
										0
									],
									"source": [
										"obj-1",
										3
									]
								}
							}
						]
					}
				}
			},
			{
				"box": {
					"id": "obj-snap-s",
					"maxclass": "newobj",
					"text": "snapshot~ 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						130.0,
						160.0,
						124.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-snap-v",
					"maxclass": "newobj",
					"text": "snapshot~ 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						240.0,
						160.0,
						124.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-pack",
					"maxclass": "newobj",
					"text": "pack /fohmixer/lufs 2 0 v1.58.0 - - - - -700 -700 0",
					"numinlets": 11,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						30.0,
						460.0,
						440.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-udpsend",
					"maxclass": "newobj",
					"text": "udpsend 127.0.0.1 39110",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						30.0,
						540.0,
						204.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-times-s",
					"maxclass": "newobj",
					"text": "* 10.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						220.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-round-s",
					"maxclass": "newobj",
					"text": "round 1.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						260.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-clip-s",
					"maxclass": "newobj",
					"text": "clip -700 0",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						300.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-times-v",
					"maxclass": "newobj",
					"text": "* 10.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						220.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-round-v",
					"maxclass": "newobj",
					"text": "round 1.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						260.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-clip-v",
					"maxclass": "newobj",
					"text": "clip -700 0",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						300.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-short",
					"maxclass": "flonum",
					"numinlets": 1,
					"numoutlets": 2,
					"patching_rect": [
						130.0,
						640.0,
						92.0,
						36.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						40.0,
						92.0,
						36.0
					],
					"outlettype": [
						"",
						"bang"
					],
					"format": 6,
					"ignoreclick": 1,
					"minimum": -70.0,
					"maximum": 12.0,
					"parameter_enable": 0,
					"fontsize": 26.0,
					"fontface": 1,
					"numdecimalplaces": 1,
					"triscale": 0.0
				}
			},
			{
				"box": {
					"id": "obj-voice",
					"maxclass": "flonum",
					"numinlets": 1,
					"numoutlets": 2,
					"patching_rect": [
						240.0,
						640.0,
						92.0,
						36.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						40.0,
						92.0,
						36.0
					],
					"outlettype": [
						"",
						"bang"
					],
					"format": 6,
					"ignoreclick": 1,
					"minimum": -70.0,
					"maximum": 12.0,
					"parameter_enable": 0,
					"fontsize": 26.0,
					"fontface": 1,
					"numdecimalplaces": 1,
					"triscale": 0.0
				}
			},
			{
				"box": {
					"id": "obj-caption",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						350.0,
						640.0,
						60.0,
						20.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						2.0,
						60.0,
						20.0
					],
					"text": "LUFS",
					"fontsize": 14.0,
					"fontface": 1
				}
			},
			{
				"box": {
					"id": "obj-label-l",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						130.0,
						600.0,
						92.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						22.0,
						92.0,
						18.0
					],
					"text": "Short-term",
					"fontsize": 11.0
				}
			},
			{
				"box": {
					"id": "obj-label-r",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						240.0,
						600.0,
						92.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						22.0,
						92.0,
						18.0
					],
					"text": "Voice",
					"fontsize": 11.0
				}
			},
			{
				"box": {
					"id": "obj-title",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						350.0,
						600.0,
						120.0,
						16.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						80.0,
						120.0,
						16.0
					],
					"text": "FohMixer LUFS",
					"fontsize": 9.0
				}
			},
			{
				"box": {
					"id": "obj-version",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						460.0,
						600.0,
						60.0,
						16.0
					],
					"presentation": 1,
					"presentation_rect": [
						140.0,
						80.0,
						60.0,
						16.0
					],
					"text": "v1.58.0",
					"varname": "version",
					"fontsize": 9.0
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
						420.0,
						20.0,
						140.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						100.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-path",
					"maxclass": "message",
					"text": "path this_device",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						720.0,
						220.0,
						148.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						720.0,
						260.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-route",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						720.0,
						300.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						360.0,
						284.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						420.0,
						400.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						420.0,
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-t",
					"maxclass": "newobj",
					"text": "t i i i",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"int",
						"int",
						"int"
					],
					"patching_rect": [
						420.0,
						480.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						420.0,
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-observe",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						490.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						580.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						420.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						490.0,
						560.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-changed",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						490.0,
						600.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						420.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						680.0,
						28.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						490.0,
						680.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						490.0,
						720.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						490.0,
						760.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						420.0,
						800.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						840.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						680.0,
						360.0,
						420.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						680.0,
						400.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						680.0,
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-t",
					"maxclass": "newobj",
					"text": "t i i i",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"int",
						"int",
						"int"
					],
					"patching_rect": [
						680.0,
						480.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						680.0,
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-observe",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						750.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						840.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						680.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						750.0,
						560.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-changed",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						750.0,
						600.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						680.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						680.0,
						680.0,
						28.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						750.0,
						680.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						750.0,
						720.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						750.0,
						760.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						680.0,
						800.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						680.0,
						840.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent canonical_parent canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						940.0,
						360.0,
						556.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						940.0,
						400.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						940.0,
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-t",
					"maxclass": "newobj",
					"text": "t i i i",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"int",
						"int",
						"int"
					],
					"patching_rect": [
						940.0,
						480.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						940.0,
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-observe",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1010.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1100.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						940.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1010.0,
						560.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-changed",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						1010.0,
						600.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						940.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						940.0,
						680.0,
						28.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1010.0,
						680.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1010.0,
						720.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1010.0,
						760.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						940.0,
						800.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						940.0,
						840.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent canonical_parent canonical_parent canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						360.0,
						692.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						1200.0,
						400.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1200.0,
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-t",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						1200.0,
						480.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						1200.0,
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1360.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						1200.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						1200.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						680.0,
						28.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1270.0,
						680.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1270.0,
						720.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1270.0,
						760.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1200.0,
						800.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						840.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-send-trigger",
					"maxclass": "newobj",
					"text": "t b f",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"float"
					],
					"patching_rect": [
						30.0,
						420.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-load",
					"maxclass": "newobj",
					"text": "t b b b b b b b b b b",
					"numinlets": 1,
					"numoutlets": 10,
					"outlettype": [
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						420.0,
						140.0,
						200.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-property",
					"maxclass": "message",
					"text": "property name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						820.0,
						180.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-offset",
					"maxclass": "newobj",
					"text": "random 10000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						1200.0,
						60.0,
						90.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-offset-delay",
					"maxclass": "newobj",
					"text": "delay",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						1200.0,
						100.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-clock-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						140.0,
						58.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-clock-start",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						180.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-clock",
					"maxclass": "newobj",
					"text": "qmetro 10000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						1200.0,
						220.0,
						90.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-tick",
					"maxclass": "newobj",
					"text": "t b b b b b",
					"numinlets": 1,
					"numoutlets": 5,
					"outlettype": [
						"bang",
						"bang",
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						1200.0,
						260.0,
						120.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-pull-gate",
					"maxclass": "newobj",
					"text": "gate",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						100.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-pull",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						130.0,
						130.0,
						51.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-receive",
					"maxclass": "newobj",
					"text": "udpreceive 40000",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1500.0,
						100.0,
						128.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-route",
					"maxclass": "newobj",
					"text": "route /fohmixer/lufs/poll",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1500.0,
						140.0,
						191.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-draw",
					"maxclass": "newobj",
					"text": "random 10000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1500.0,
						20.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-offset",
					"maxclass": "newobj",
					"text": "+ 40000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						1500.0,
						50.0,
						65.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-port",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						1620.0,
						50.0,
						51.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-set",
					"maxclass": "newobj",
					"text": "prepend port",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1620.0,
						80.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-property",
					"maxclass": "message",
					"text": "property is_active",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1500.0,
						200.0,
						142.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-path",
					"maxclass": "message",
					"text": "path this_device",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1680.0,
						200.0,
						128.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						1680.0,
						240.0,
						79.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1500.0,
						280.0,
						107.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-t",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						1500.0,
						320.0,
						51.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-sel",
					"maxclass": "newobj",
					"text": "sel 1",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						1500.0,
						360.0,
						51.0,
						22.0
					]
				}
			}
		],
		"lines": [
			{
				"patchline": {
					"source": [
						"obj-silence",
						0
					],
					"destination": [
						"obj-plugout",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-silence",
						0
					],
					"destination": [
						"obj-plugout",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-plugin",
						0
					],
					"destination": [
						"obj-gen",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-plugin",
						1
					],
					"destination": [
						"obj-gen",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-gen",
						2
					],
					"destination": [
						"obj-snap-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-gen",
						3
					],
					"destination": [
						"obj-snap-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-s",
						0
					],
					"destination": [
						"obj-times-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-times-s",
						0
					],
					"destination": [
						"obj-round-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-round-s",
						0
					],
					"destination": [
						"obj-clip-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-s",
						0
					],
					"destination": [
						"obj-pack",
						8
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-v",
						0
					],
					"destination": [
						"obj-times-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-times-v",
						0
					],
					"destination": [
						"obj-round-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-round-v",
						0
					],
					"destination": [
						"obj-clip-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-v",
						0
					],
					"destination": [
						"obj-send-trigger",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-s",
						0
					],
					"destination": [
						"obj-short",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-v",
						0
					],
					"destination": [
						"obj-voice",
						0
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
						"obj-id-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-path",
						0
					],
					"destination": [
						"obj-id-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-live-path",
						0
					],
					"destination": [
						"obj-id-route",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-route",
						0
					],
					"destination": [
						"obj-pack",
						2
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-path",
						0
					],
					"destination": [
						"obj-parent1-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-path",
						0
					],
					"destination": [
						"obj-parent2-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-path",
						0
					],
					"destination": [
						"obj-parent3-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-path",
						0
					],
					"destination": [
						"obj-parent4-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-send-trigger",
						1
					],
					"destination": [
						"obj-pack",
						9
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-send-trigger",
						0
					],
					"destination": [
						"obj-pack",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-defer",
						0
					],
					"destination": [
						"obj-id-load",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						8
					],
					"destination": [
						"obj-id-property",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						7
					],
					"destination": [
						"obj-id-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						6
					],
					"destination": [
						"obj-parent1-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-live-path",
						1
					],
					"destination": [
						"obj-parent1-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						5
					],
					"destination": [
						"obj-parent2-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-live-path",
						1
					],
					"destination": [
						"obj-parent2-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						4
					],
					"destination": [
						"obj-parent3-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-live-path",
						1
					],
					"destination": [
						"obj-parent3-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						3
					],
					"destination": [
						"obj-parent4-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-live-path",
						1
					],
					"destination": [
						"obj-parent4-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-live-path",
						0
					],
					"destination": [
						"obj-parent4-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-route-id",
						0
					],
					"destination": [
						"obj-parent1-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-t",
						2
					],
					"destination": [
						"obj-parent1-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-prepend",
						0
					],
					"destination": [
						"obj-parent1-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-t",
						0
					],
					"destination": [
						"obj-parent1-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-sel",
						0
					],
					"destination": [
						"obj-parent1-empty",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-sel",
						1
					],
					"destination": [
						"obj-parent1-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-changed",
						0
					],
					"destination": [
						"obj-parent1-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-read",
						2
					],
					"destination": [
						"obj-parent1-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-empty",
						1
					],
					"destination": [
						"obj-parent1-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-read",
						1
					],
					"destination": [
						"obj-parent1-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-get",
						0
					],
					"destination": [
						"obj-parent1-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-object",
						0
					],
					"destination": [
						"obj-parent1-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-read",
						0
					],
					"destination": [
						"obj-parent1-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-empty",
						0
					],
					"destination": [
						"obj-parent1-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-route-name",
						0
					],
					"destination": [
						"obj-parent1-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-none",
						0
					],
					"destination": [
						"obj-parent1-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-reg",
						0
					],
					"destination": [
						"obj-parent1-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-symbol",
						0
					],
					"destination": [
						"obj-pack",
						4
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-t",
						1
					],
					"destination": [
						"obj-parent1-observe",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-property",
						0
					],
					"destination": [
						"obj-parent1-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-observe",
						0
					],
					"destination": [
						"obj-parent1-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-observer",
						0
					],
					"destination": [
						"obj-parent1-changed",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-route-id",
						0
					],
					"destination": [
						"obj-parent2-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-t",
						2
					],
					"destination": [
						"obj-parent2-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-prepend",
						0
					],
					"destination": [
						"obj-parent2-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-t",
						0
					],
					"destination": [
						"obj-parent2-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-sel",
						0
					],
					"destination": [
						"obj-parent2-empty",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-sel",
						1
					],
					"destination": [
						"obj-parent2-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-changed",
						0
					],
					"destination": [
						"obj-parent2-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-read",
						2
					],
					"destination": [
						"obj-parent2-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-empty",
						1
					],
					"destination": [
						"obj-parent2-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-read",
						1
					],
					"destination": [
						"obj-parent2-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-get",
						0
					],
					"destination": [
						"obj-parent2-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-object",
						0
					],
					"destination": [
						"obj-parent2-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-read",
						0
					],
					"destination": [
						"obj-parent2-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-empty",
						0
					],
					"destination": [
						"obj-parent2-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-route-name",
						0
					],
					"destination": [
						"obj-parent2-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-none",
						0
					],
					"destination": [
						"obj-parent2-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-reg",
						0
					],
					"destination": [
						"obj-parent2-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-symbol",
						0
					],
					"destination": [
						"obj-pack",
						5
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-t",
						1
					],
					"destination": [
						"obj-parent2-observe",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-property",
						0
					],
					"destination": [
						"obj-parent2-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-observe",
						0
					],
					"destination": [
						"obj-parent2-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-observer",
						0
					],
					"destination": [
						"obj-parent2-changed",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-route-id",
						0
					],
					"destination": [
						"obj-parent3-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-t",
						2
					],
					"destination": [
						"obj-parent3-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-prepend",
						0
					],
					"destination": [
						"obj-parent3-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-t",
						0
					],
					"destination": [
						"obj-parent3-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-sel",
						0
					],
					"destination": [
						"obj-parent3-empty",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-sel",
						1
					],
					"destination": [
						"obj-parent3-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-changed",
						0
					],
					"destination": [
						"obj-parent3-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-read",
						2
					],
					"destination": [
						"obj-parent3-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-empty",
						1
					],
					"destination": [
						"obj-parent3-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-read",
						1
					],
					"destination": [
						"obj-parent3-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-get",
						0
					],
					"destination": [
						"obj-parent3-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-object",
						0
					],
					"destination": [
						"obj-parent3-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-read",
						0
					],
					"destination": [
						"obj-parent3-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-empty",
						0
					],
					"destination": [
						"obj-parent3-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-route-name",
						0
					],
					"destination": [
						"obj-parent3-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-none",
						0
					],
					"destination": [
						"obj-parent3-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-reg",
						0
					],
					"destination": [
						"obj-parent3-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-symbol",
						0
					],
					"destination": [
						"obj-pack",
						6
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-t",
						1
					],
					"destination": [
						"obj-parent3-observe",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-property",
						0
					],
					"destination": [
						"obj-parent3-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-observe",
						0
					],
					"destination": [
						"obj-parent3-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-observer",
						0
					],
					"destination": [
						"obj-parent3-changed",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-route-id",
						0
					],
					"destination": [
						"obj-parent4-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-t",
						1
					],
					"destination": [
						"obj-parent4-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-prepend",
						0
					],
					"destination": [
						"obj-parent4-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-t",
						0
					],
					"destination": [
						"obj-parent4-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-sel",
						0
					],
					"destination": [
						"obj-parent4-empty",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-sel",
						1
					],
					"destination": [
						"obj-parent4-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-read",
						2
					],
					"destination": [
						"obj-parent4-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-empty",
						1
					],
					"destination": [
						"obj-parent4-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-read",
						1
					],
					"destination": [
						"obj-parent4-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-get",
						0
					],
					"destination": [
						"obj-parent4-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-object",
						0
					],
					"destination": [
						"obj-parent4-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-read",
						0
					],
					"destination": [
						"obj-parent4-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-empty",
						0
					],
					"destination": [
						"obj-parent4-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-route-name",
						0
					],
					"destination": [
						"obj-parent4-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-none",
						0
					],
					"destination": [
						"obj-parent4-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-reg",
						0
					],
					"destination": [
						"obj-parent4-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-symbol",
						0
					],
					"destination": [
						"obj-pack",
						7
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
						"obj-id-offset",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-offset",
						0
					],
					"destination": [
						"obj-id-offset-delay",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-offset-delay",
						0
					],
					"destination": [
						"obj-id-clock-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-clock-defer",
						0
					],
					"destination": [
						"obj-id-clock-start",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-clock-start",
						0
					],
					"destination": [
						"obj-id-clock",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-clock",
						0
					],
					"destination": [
						"obj-id-tick",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-tick",
						4
					],
					"destination": [
						"obj-parent1-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-tick",
						3
					],
					"destination": [
						"obj-parent2-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-tick",
						2
					],
					"destination": [
						"obj-parent3-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-tick",
						1
					],
					"destination": [
						"obj-parent4-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-live-path",
						0
					],
					"destination": [
						"obj-parent1-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-live-path",
						0
					],
					"destination": [
						"obj-parent2-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-live-path",
						0
					],
					"destination": [
						"obj-parent3-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pack",
						0
					],
					"destination": [
						"obj-udpsend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull-gate",
						0
					],
					"destination": [
						"obj-pull",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull",
						1
					],
					"destination": [
						"obj-snap-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull",
						0
					],
					"destination": [
						"obj-snap-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-receive",
						0
					],
					"destination": [
						"obj-poll-route",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-route",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						9
					],
					"destination": [
						"obj-poll-draw",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-draw",
						0
					],
					"destination": [
						"obj-poll-offset",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-offset",
						0
					],
					"destination": [
						"obj-poll-port",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-port",
						1
					],
					"destination": [
						"obj-poll-set",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-set",
						0
					],
					"destination": [
						"obj-poll-receive",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-port",
						0
					],
					"destination": [
						"obj-pack",
						10
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						2
					],
					"destination": [
						"obj-active-property",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						1
					],
					"destination": [
						"obj-active-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-property",
						0
					],
					"destination": [
						"obj-active-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-path",
						0
					],
					"destination": [
						"obj-active-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-live-path",
						0
					],
					"destination": [
						"obj-active-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-observer",
						0
					],
					"destination": [
						"obj-active-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-t",
						1
					],
					"destination": [
						"obj-pull-gate",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-t",
						0
					],
					"destination": [
						"obj-active-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-sel",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-tick",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			}
		],
		"dependency_cache": [],
		"autosave": 0
	}
}
