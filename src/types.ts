export type Alias={enabled:boolean;phrase:string;action:string;payload:string};
export type Builtin={id:string;title:string;action:string;phrases:string[];payload:string};
export type BuiltinOverride={id:string;enabled:boolean;phrases:string[];action:string;payload:string};
export type VoicePreset={name:string;engine:string;voice:string;model:string;speaker:number;rate:number;fish_voice:string;fish_model:string};
export type Settings={
 onboarding_complete:boolean;wake_words:string[];activation:'wake'|'hotkey'|'both';hotkey:string;microphone:string;
 vosk_library:string;vosk_model:string;speech_mode:'vosk'|'whisper';whisper_executable:string;whisper_model:string;
 speech_language:string;speech_hint:string;speech_gain:number;noise_floor:number;wake_aliases:string[];speech_corrections:{heard:string;replacement:string}[];
 llm_repair_speech:boolean;web_search_url:string;
 music_service:string;browser:string;music_mode:'desktop'|'browser';yandex_app_path:string;spotify_app_path:string;
 island_enabled:boolean;llm_fallback:boolean;voice_feedback:boolean;start_muted:boolean;start_in_tray:boolean;command_timeout:number;
 tts_engine:string;tts_voice:string;tts_volume:number;tts_rate:number;piper_executable:string;piper_model:string;piper_speaker:number;voice_presets:VoicePreset[];
 fish_voice:string;fish_model:string;
 llm_provider:'lm_studio'|'ollama'|'open_router'|'open_ai'|'compatible';llm_url:string;llm_model:string;llm_structured:boolean;llm_style:'neutral'|'casual'|'bold';
 aliases:Alias[];builtin_overrides:BuiltinOverride[];
};
