use super::*;
use std::io::{BufReader, Cursor, Read};
use std::net::TcpListener;

fn tcp_stream(dho: &DHO5108) -> &TcpStream {
    match &dho.transport {
        DhoTransport::Tcp(reader) => reader.get_ref(),
        #[cfg(all(target_os = "windows", feature = "gpib"))]
        DhoTransport::Visa(_) => panic!("test instrument unexpectedly uses VISA"),
    }
}

#[test]
fn binary_block_length_skips_leftover_line_terminators() {
    let cursor = Cursor::new(b"\r\n#14abcd".to_vec());
    let mut reader = BufReader::new(cursor);

    let length = read_binary_block_length(&mut reader).unwrap();
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).unwrap();

    assert_eq!(length, 4);
    assert_eq!(payload, b"abcd");
}

#[test]
fn binary_block_length_rejects_non_block_data() {
    let cursor = Cursor::new(b"not a block".to_vec());
    let mut reader = BufReader::new(cursor);

    let error = read_binary_block_length(&mut reader).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn memory_depth_parser_accepts_scientific_notation() {
    assert_eq!(parse_memory_depth("1.000E+6").unwrap(), 1_000_000);
    assert_eq!(parse_memory_depth("200000000").unwrap(), 200_000_000);
}

#[test]
fn memory_depth_parser_rejects_non_integer_values() {
    assert!(parse_memory_depth("AUTO").is_err());
    assert!(parse_memory_depth("1.5").is_err());
    assert!(parse_memory_depth("0").is_err());
    assert!(parse_memory_depth(&usize::MAX.to_string()).is_err());
}

#[test]
fn waveform_number_parser_rejects_non_finite_values() {
    assert_eq!(
        parse_finite_f64(" 2.693333e-05 ", "yincrement").unwrap(),
        2.693333e-05
    );
    assert!(parse_finite_f64("NaN", "yincrement").is_err());
    assert!(parse_finite_f64("inf", "yincrement").is_err());
}

#[test]
fn raw_word_size_validation_requires_exact_payload() {
    assert_eq!(expected_raw_word_bytes(2).unwrap(), 4);
    validate_binary_block_length(4, 4).unwrap();
    assert!(validate_binary_block_length(3, 4).is_err());
    assert!(validate_binary_block_length(5, 4).is_err());
    assert!(expected_raw_word_bytes(usize::MAX).is_err());
}

#[test]
fn raw_word_setup_stops_acquisition_before_selecting_raw_data() {
    let commands = raw_word_setup_commands(3, 200_000_000);

    assert_eq!(commands[0], ":STOP");
    assert_eq!(commands[1], "WAV:SOUR CHAN3");
    assert_eq!(commands[2], "WAV:MODE RAW");
    assert_eq!(commands[3], "WAV:FORM WORD");
    assert_eq!(commands.last().unwrap(), "*OPC?");
}

#[test]
fn opc_response_must_report_completion() {
    validate_opc_response("1").unwrap();
    validate_opc_response(" 1 ").unwrap();
    assert!(validate_opc_response("").is_err());
    assert!(validate_opc_response("0").is_err());
    assert!(validate_opc_response("ready").is_err());
}

#[test]
fn trigger_status_parser_accepts_documented_states() {
    assert_eq!(
        parse_trigger_status("TD").unwrap(),
        DhoTriggerStatus::Triggered
    );
    assert_eq!(
        parse_trigger_status("WAIT").unwrap(),
        DhoTriggerStatus::Wait
    );
    assert_eq!(parse_trigger_status("RUN").unwrap(), DhoTriggerStatus::Run);
    assert_eq!(
        parse_trigger_status("AUTO").unwrap(),
        DhoTriggerStatus::Auto
    );
    assert_eq!(
        parse_trigger_status("STOP").unwrap(),
        DhoTriggerStatus::Stop
    );
    assert!(parse_trigger_status("UNKNOWN").is_err());
}

#[test]
fn raw_fetch_verifies_state_and_accepts_fragmented_binary_block() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);

        for expected in raw_word_setup_commands(1, 2) {
            expect_command(&mut reader, &expected);
        }
        reply_line(&mut reader, "1");
        for (command, response) in [
            (":TRIGger:STATus?", "STOP"),
            (":WAVeform:SOURce?", "CHAN1"),
            (":WAVeform:MODE?", "RAW"),
            (":WAVeform:FORMat?", "WORD"),
            (":WAVeform:POINts?", "2"),
            ("WAV:PRE?", "1,2,2,1,0.5,0,0,1,0,0"),
            ("WAV:XINC?", "0.5"),
            ("WAV:XOR?", "0"),
            ("WAV:XREF?", "0"),
            ("WAV:YINC?", "1"),
            ("WAV:YOR?", "0"),
            ("WAV:YREF?", "0"),
            (":CHANnel1:OFFSet?", "0"),
            (":CHANnel1:SCALe?", "1"),
        ] {
            expect_command(&mut reader, command);
            reply_line(&mut reader, response);
        }
        expect_command(&mut reader, "WAV:DATA?");
        for chunk in [b"#".as_slice(), b"1", b"4", b"\x01", b"\x00\x02", b"\x00\n"] {
            reader.get_mut().write_all(chunk).unwrap();
            reader.get_mut().flush().unwrap();
        }
        expect_command(&mut reader, ":TRIGger:STATus?");
        reply_line(&mut reader, "STOP");
    });

    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(1)),
        Some(Duration::from_secs(1)),
    )
    .unwrap();
    let mut output = Vec::new();
    let result = dho.fetch_raw_word_into(1, 2, &mut output).unwrap();

    assert_eq!(result.byte_count, 4);
    assert_eq!(output, [1, 0, 2, 0]);
    server.join().unwrap();
}

#[test]
fn raw_setup_polls_stale_source_without_resending_commands() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);

        for expected in raw_word_setup_commands(1, 2) {
            expect_command(&mut reader, &expected);
        }
        reply_line(&mut reader, "1");
        // A complete source readback may remain stale after OPC. Poll it;
        // do not send another STOP, source setter, or setup burst.
        for (command, response) in [
            (":TRIGger:STATus?", "STOP"),
            (":WAVeform:SOURce?", "CHAN3"),
            (":TRIGger:STATus?", "STOP"),
            (":WAVeform:SOURce?", "CHAN3"),
            (":TRIGger:STATus?", "STOP"),
            (":WAVeform:SOURce?", "CHAN1"),
            (":WAVeform:MODE?", "RAW"),
            (":WAVeform:FORMat?", "WORD"),
            (":WAVeform:POINts?", "2"),
            ("WAV:PRE?", "1,2,2,1,0.5,0,0,1,0,0"),
            ("WAV:XINC?", "0.5"),
            ("WAV:XOR?", "0"),
            ("WAV:XREF?", "0"),
            ("WAV:YINC?", "1"),
            ("WAV:YOR?", "0"),
            ("WAV:YREF?", "0"),
            (":CHANnel1:OFFSet?", "0"),
            (":CHANnel1:SCALe?", "1"),
        ] {
            expect_command(&mut reader, command);
            reply_line(&mut reader, response);
        }
        expect_command(&mut reader, "WAV:DATA?");
        reader
            .get_mut()
            .write_all(b"#14\x01\x00\x02\x00\n")
            .unwrap();
        reader.get_mut().flush().unwrap();
        expect_command(&mut reader, ":TRIGger:STATus?");
        reply_line(&mut reader, "STOP");
    });

    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(2)),
        Some(Duration::from_secs(2)),
    )
    .unwrap();
    let mut output = Vec::new();
    let result = dho.fetch_raw_word_into(1, 2, &mut output).unwrap();

    assert_eq!(result.byte_count, 4);
    assert_eq!(output, [1, 0, 2, 0]);
    server.join().unwrap();
}

fn assert_raw_setup_failure_without_further_commands(
    opc_reply: Option<&[u8]>,
    replies: &[(&str, &[u8])],
    expected_kinds: &[io::ErrorKind],
    expected_message: Option<&str>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let opc_reply = opc_reply.map(<[u8]>::to_vec);
    let replies: Vec<_> = replies
        .iter()
        .map(|(command, reply)| (command.to_string(), reply.to_vec()))
        .collect();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        for expected in raw_word_setup_commands(1, 2) {
            expect_command(&mut reader, &expected);
        }
        match opc_reply {
            Some(reply) => {
                reader.get_mut().write_all(&reply).unwrap();
                reader.get_mut().flush().unwrap();
            }
            None => reader
                .get_mut()
                .shutdown(std::net::Shutdown::Write)
                .unwrap(),
        }
        for (command, reply) in replies {
            expect_command(&mut reader, &command);
            reader.get_mut().write_all(&reply).unwrap();
            reader.get_mut().flush().unwrap();
        }
        let mut extra_commands = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            extra_commands.push(line.trim_end().to_string());
        }
        extra_commands
    });
    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(1)),
        Some(Duration::from_millis(100)),
    )
    .unwrap();
    let mut output = vec![0xa5];
    let error = dho.fetch_raw_word_into(1, 2, &mut output).unwrap_err();
    drop(dho);
    let extra_commands = server.join().unwrap();
    assert!(expected_kinds.contains(&error.kind()), "{error:?}");
    if let Some(message) = expected_message {
        assert!(error.to_string().contains(message), "{error}");
    }
    assert_eq!(output, [0xa5]);
    assert!(extra_commands.is_empty(), "{extra_commands:?}");
}

#[test]
fn raw_setup_propagates_opc_timeout_without_replaying_writes() {
    assert_raw_setup_failure_without_further_commands(
        Some(b""),
        &[],
        &[io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock],
        None,
    );
}

#[test]
fn raw_setup_propagates_partial_source_timeout_without_further_queries() {
    assert_raw_setup_failure_without_further_commands(
        Some(b"1\n"),
        &[
            (":TRIGger:STATus?", b"STOP\n"),
            (":WAVeform:SOURce?", b"CHAN"),
        ],
        &[io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock],
        None,
    );
}

#[test]
fn raw_setup_does_not_replay_after_eof() {
    assert_raw_setup_failure_without_further_commands(
        None,
        &[],
        &[io::ErrorKind::UnexpectedEof],
        Some("newline terminator"),
    );
}

#[test]
fn raw_setup_does_not_replay_after_invalid_opc() {
    for reply in [b"0\n".as_slice(), b"ready\n".as_slice()] {
        assert_raw_setup_failure_without_further_commands(
            Some(reply),
            &[],
            &[io::ErrorKind::InvalidData],
            Some("invalid *OPC? response"),
        );
    }
}

#[test]
fn raw_setup_rejects_unterminated_source_at_eof_without_further_queries() {
    for source in [b"CHAN3".as_slice(), b"CHAN1".as_slice()] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let source = source.to_vec();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            for expected in raw_word_setup_commands(1, 2) {
                expect_command(&mut reader, &expected);
            }
            reply_line(&mut reader, "1");
            expect_command(&mut reader, ":TRIGger:STATus?");
            reply_line(&mut reader, "STOP");
            expect_command(&mut reader, ":WAVeform:SOURce?");
            reader.get_mut().write_all(&source).unwrap();
            reader.get_mut().flush().unwrap();
            reader
                .get_mut()
                .shutdown(std::net::Shutdown::Write)
                .unwrap();
            let mut extra_commands = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    return extra_commands;
                }
                extra_commands.push(line.trim_end().to_string());
            }
        });
        let mut dho = DHO5108::open_with_timeouts(
            "127.0.0.1",
            port,
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(1)),
        )
        .unwrap();
        let mut output = vec![0xa5];
        let error = dho.fetch_raw_word_into(1, 2, &mut output).unwrap_err();
        drop(dho);
        let extra_commands = server.join().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "{error}");
        assert_eq!(output, [0xa5]);
        assert!(extra_commands.is_empty(), "{extra_commands:?}");
    }
}

#[test]
fn raw_setup_aborts_if_acquisition_is_not_stopped() {
    for status in [b"RUN\n".as_slice(), b"WAIT\n", b"AUTO\n", b"TD\n"] {
        assert_raw_setup_failure_without_further_commands(
            Some(b"1\n"),
            &[(":TRIGger:STATus?", status)],
            &[io::ErrorKind::InvalidData],
            Some("unexpected trigger status"),
        );
    }
}

#[test]
fn raw_setup_aborts_if_acquisition_restarts_while_source_is_pending() {
    assert_raw_setup_failure_without_further_commands(
        Some(b"1\n"),
        &[
            (":TRIGger:STATus?", b"STOP\n"),
            (":WAVeform:SOURce?", b"CHAN3\n"),
            (":TRIGger:STATus?", b"RUN\n"),
        ],
        &[io::ErrorKind::InvalidData],
        Some("unexpected trigger status"),
    );
}

#[test]
fn raw_setup_propagates_io_failure_while_polling() {
    assert_raw_setup_failure_without_further_commands(
        Some(b"1\n"),
        &[
            (":TRIGger:STATus?", b"STOP\n"),
            (":WAVeform:SOURce?", b"CHAN3\n"),
            (":TRIGger:STATus?", b""),
        ],
        &[io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock],
        None,
    );
}

#[test]
fn raw_setup_rejects_malformed_source_without_polling() {
    for source in [b"invalid\n".as_slice(), b"CHAN9\n", b"1\n"] {
        assert_raw_setup_failure_without_further_commands(
            Some(b"1\n"),
            &[
                (":TRIGger:STATus?", b"STOP\n"),
                (":WAVeform:SOURce?", source),
            ],
            &[io::ErrorKind::InvalidData],
            Some("unexpected waveform source"),
        );
    }
}

#[test]
fn raw_setup_keeps_other_waveform_checks_strict() {
    for (last_command, last_reply, message) in [
        (
            ":WAVeform:MODE?",
            b"NORM\n".as_slice(),
            "unexpected waveform mode",
        ),
        (
            ":WAVeform:FORMat?",
            b"BYTE\n".as_slice(),
            "unexpected waveform format",
        ),
        (
            ":WAVeform:POINts?",
            b"1\n".as_slice(),
            "waveform points are 1, expected 2",
        ),
    ] {
        let mut replies: Vec<(&str, &[u8])> = vec![
            (":TRIGger:STATus?", b"STOP\n"),
            (":WAVeform:SOURce?", b"CHAN1\n"),
        ];
        if last_command != ":WAVeform:MODE?" {
            replies.push((":WAVeform:MODE?", b"RAW\n"));
        }
        if last_command == ":WAVeform:POINts?" {
            replies.push((":WAVeform:FORMat?", b"WORD\n"));
        }
        replies.push((last_command, last_reply));
        assert_raw_setup_failure_without_further_commands(
            Some(b"1\n"),
            &replies,
            &[io::ErrorKind::InvalidData],
            Some(message),
        );
    }
}

#[test]
fn raw_setup_stale_source_times_out_without_resending_or_writing_payload() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        for expected in raw_word_setup_commands(1, 2) {
            expect_command(&mut reader, &expected);
        }
        reply_line(&mut reader, "1");
        let mut polls = 0;
        loop {
            let mut command = String::new();
            if reader.read_line(&mut command).unwrap() == 0 {
                return polls;
            }
            assert_eq!(command.trim_end(), ":TRIGger:STATus?");
            reply_line(&mut reader, "STOP");
            expect_command(&mut reader, ":WAVeform:SOURce?");
            reply_line(&mut reader, "CHAN3");
            polls += 1;
        }
    });
    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(1)),
        Some(Duration::from_secs(1)),
    )
    .unwrap();
    let started = Instant::now();
    let mut output = vec![0xa5];
    let error = dho.fetch_raw_word_into(1, 2, &mut output).unwrap_err();
    let elapsed = started.elapsed();
    drop(dho);
    assert!(server.join().unwrap() > 0);
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("CHAN3"));
    assert!(error.to_string().contains("CHAN1"));
    assert!(error.to_string().contains("did not settle within"));
    assert_eq!(output, [0xa5]);
    assert!(elapsed >= RAW_SOURCE_SETTLE_TIMEOUT);
    assert!(elapsed < Duration::from_secs(5));
}

#[test]
fn tcp_query_respects_idle_timeout() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        expect_command(&mut reader, "*IDN?");
        std::thread::sleep(Duration::from_millis(100));
    });
    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(1)),
        Some(Duration::from_millis(20)),
    )
    .unwrap();

    let error = dho.identify().unwrap_err();

    assert!(matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ));
    server.join().unwrap();
}

#[test]
fn binary_query_rejects_declared_payload_length_mismatch() {
    for response in [b"#13abc".as_slice(), b"#15abcde".as_slice()] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let response = response.to_vec();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            expect_command(&mut reader, "BIN?");
            reader.get_mut().write_all(&response).unwrap();
            reader.get_mut().flush().unwrap();
        });
        let mut dho = DHO5108::open_with_timeouts(
            "127.0.0.1",
            port,
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(1)),
        )
        .unwrap();
        let mut output = Vec::new();

        let error = dho
            .query_binary_into_with_expected_length("BIN?", &mut output, Some(4))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("expected 4 bytes"));
        assert!(output.is_empty());
        server.join().unwrap();
    }
}

#[test]
fn binary_query_rejects_truncated_payload() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        expect_command(&mut reader, "BIN?");
        reader.get_mut().write_all(b"#14abc").unwrap();
        reader.get_mut().flush().unwrap();
    });
    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(1)),
        Some(Duration::from_secs(1)),
    )
    .unwrap();
    let mut output = Vec::new();

    let error = dho.query_binary_into("BIN?", &mut output).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert!(error.to_string().contains("expected 4"));
    assert_eq!(output, b"abc");
    server.join().unwrap();
}

#[test]
fn binary_query_without_terminator_allows_a_following_query() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        expect_command(&mut reader, "BIN?");
        reader.get_mut().write_all(b"#14abcd").unwrap();
        reader.get_mut().flush().unwrap();
        expect_command(&mut reader, "*IDN?");
        reply_line(&mut reader, "RIGOL,DHO5108,serial,firmware");
    });
    let mut dho = DHO5108::open_with_timeouts(
        "127.0.0.1",
        port,
        Some(Duration::from_secs(1)),
        Some(Duration::from_secs(1)),
    )
    .unwrap();

    assert_eq!(dho.query_binary("BIN?").unwrap(), b"abcd");
    assert_eq!(dho.identify().unwrap(), "RIGOL,DHO5108,serial,firmware");
    server.join().unwrap();
}

fn expect_command(reader: &mut BufReader<TcpStream>, expected: &str) {
    let mut command = String::new();
    reader.read_line(&mut command).unwrap();
    assert_eq!(command.trim_end(), expected);
}

fn reply_line(reader: &mut BufReader<TcpStream>, response: &str) {
    writeln!(reader.get_mut(), "{response}").unwrap();
    reader.get_mut().flush().unwrap();
}

#[test]
fn display_png_preserves_unlimited_timeout_and_allows_following_query() {
    let image = b"\x89PNG\r\n\x1a\npayload".to_vec();
    let expected_image = image.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut command = String::new();
        reader.read_line(&mut command).unwrap();
        assert_eq!(command.trim_end(), ":DISPlay:DATA? PNG");
        write!(
            reader.get_mut(),
            "#{}{}",
            expected_image.len().to_string().len(),
            expected_image.len()
        )
        .unwrap();
        reader.get_mut().write_all(&expected_image).unwrap();
        reader.get_mut().write_all(b"\n").unwrap();
        reader.get_mut().flush().unwrap();

        command.clear();
        reader.read_line(&mut command).unwrap();
        assert_eq!(command.trim_end(), ":ACQuire:MDEPth?");
        reader.get_mut().write_all(b"200000000\n").unwrap();
        reader.get_mut().flush().unwrap();
    });
    let mut dho = DHO5108::open("127.0.0.1", port, None).unwrap();

    assert_eq!(tcp_stream(&dho).read_timeout().unwrap(), None);
    assert_eq!(tcp_stream(&dho).write_timeout().unwrap(), None);
    assert_eq!(dho.capture_display_png().unwrap(), image);
    assert_eq!(tcp_stream(&dho).read_timeout().unwrap(), None);
    assert_eq!(tcp_stream(&dho).write_timeout().unwrap(), None);
    assert_eq!(dho.query_memory_depth().unwrap(), 200_000_000);
    server.join().unwrap();
}
