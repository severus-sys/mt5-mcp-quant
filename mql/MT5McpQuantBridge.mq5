#property service
#property copyright "MT5-MCP-Quant contributors"
#property link      "https://github.com/severus-sys/mt5-mcp-quant"
#property version   "1.00"
#property description "Allowlisted FILE_COMMON bridge for Market Watch and calendar export"

#define PROTOCOL_VERSION "1"
#define SERVICE_VERSION  "1.0.0"
#define ROOT_PREFIX      "mt5-mcp-quant\\bridge\\v1\\"

string g_instance_id;
string g_root;
datetime g_last_heartbeat=0;

string NormalizePath(string value)
  {
   StringReplace(value,"/","\\");
   StringToLower(value);
   while(StringLen(value)>0 && StringSubstr(value,StringLen(value)-1)=="\\")
      value=StringSubstr(value,0,StringLen(value)-1);
   return value;
  }

string TerminalInstanceId()
  {
   string value=NormalizePath(TerminalInfoString(TERMINAL_DATA_PATH));
   ulong hash=0xCBF29CE484222325;
   for(int index=0; index<StringLen(value); index++)
     {
      hash^=(uchar)StringGetCharacter(value,index);
      hash*=0x100000001B3;
     }
   return StringFormat("%016I64X",hash);
  }

bool IsSafeId(const string value)
  {
   int length=StringLen(value);
   if(length<1 || length>64)
      return false;
   for(int index=0; index<length; index++)
     {
      ushort character=StringGetCharacter(value,index);
      bool safe=(character>='a' && character<='z') ||
                (character>='A' && character<='Z') ||
                (character>='0' && character<='9') ||
                character=='-' || character=='_';
      if(!safe)
         return false;
     }
   return true;
  }

string EncodeValue(string value)
  {
   StringReplace(value,"%","%25");
   StringReplace(value,"\r","%0D");
   StringReplace(value,"\n","%0A");
   StringReplace(value,"=","%3D");
   StringReplace(value,"|","%7C");
   return value;
  }

string DecodeValue(string value)
  {
   StringReplace(value,"%7C","|");
   StringReplace(value,"%3D","=");
   StringReplace(value,"%0A","\n");
   StringReplace(value,"%0D","\r");
   StringReplace(value,"%25","%");
   return value;
  }

bool ReadFields(const string path,string &keys[],string &values[])
  {
   ArrayResize(keys,0);
   ArrayResize(values,0);
   int handle=FileOpen(path,FILE_READ|FILE_TXT|FILE_ANSI|FILE_COMMON|FILE_SHARE_READ,0,CP_UTF8);
   if(handle==INVALID_HANDLE)
      return false;
   while(!FileIsEnding(handle))
     {
      string line=FileReadString(handle);
      int split=StringFind(line,"=");
      if(split<=0)
         continue;
      int next=ArraySize(keys);
      ArrayResize(keys,next+1);
      ArrayResize(values,next+1);
      keys[next]=StringSubstr(line,0,split);
      values[next]=DecodeValue(StringSubstr(line,split+1));
     }
   FileClose(handle);
   return true;
  }

string FieldValue(const string &keys[],const string &values[],const string key)
  {
   for(int index=0; index<ArraySize(keys); index++)
      if(keys[index]==key)
         return values[index];
   return "";
  }

void PutField(string &keys[],string &values[],const string key,const string value)
  {
   int next=ArraySize(keys);
   ArrayResize(keys,next+1);
   ArrayResize(values,next+1);
   keys[next]=key;
   values[next]=value;
  }

bool WriteFieldsAtomic(const string path,const string &keys[],const string &values[])
  {
   string temporary=path+".tmp";
   int handle=FileOpen(temporary,FILE_WRITE|FILE_TXT|FILE_ANSI|FILE_COMMON|FILE_SHARE_READ,0,CP_UTF8);
   if(handle==INVALID_HANDLE)
      return false;
   for(int index=0; index<ArraySize(keys); index++)
      FileWriteString(handle,keys[index]+"="+EncodeValue(values[index])+"\n");
   FileFlush(handle);
   FileClose(handle);
   return FileMove(temporary,FILE_COMMON,path,FILE_COMMON|FILE_REWRITE);
  }

void AddContext(string &keys[],string &values[])
  {
   PutField(keys,values,"protocol",PROTOCOL_VERSION);
   PutField(keys,values,"service_version",SERVICE_VERSION);
   PutField(keys,values,"instance_id",g_instance_id);
   PutField(keys,values,"data_path",TerminalInfoString(TERMINAL_DATA_PATH));
   PutField(keys,values,"account_login",IntegerToString(AccountInfoInteger(ACCOUNT_LOGIN)));
   PutField(keys,values,"account_server",AccountInfoString(ACCOUNT_SERVER));
   PutField(keys,values,"terminal_build",IntegerToString(TerminalInfoInteger(TERMINAL_BUILD)));
   PutField(keys,values,"connected",TerminalInfoInteger(TERMINAL_CONNECTED) ? "true" : "false");
   // Protocol timestamps are Unix UTC seconds. TimeLocal() encodes the Windows
   // wall clock and is offset from Rust's SystemTime in non-UTC time zones.
   PutField(keys,values,"updated_epoch",IntegerToString((long)TimeGMT()));
  }

void WriteHeartbeat()
  {
   string keys[],values[];
   AddContext(keys,values);
   WriteFieldsAtomic(g_root+"\\heartbeat.kv",keys,values);
   g_last_heartbeat=TimeLocal();
  }

void RespondError(const string request_id,const string code,const string message)
  {
   string keys[],values[];
   AddContext(keys,values);
   PutField(keys,values,"request_id",request_id);
   PutField(keys,values,"ok","false");
   PutField(keys,values,"code",code);
   PutField(keys,values,"message",message);
   WriteFieldsAtomic(g_root+"\\responses\\"+request_id+".res",keys,values);
  }

void HandleListSymbols(const string request_id)
  {
   string symbols_temporary=g_root+"\\responses\\"+request_id+".symbols.tmp";
   string symbols_path=g_root+"\\responses\\"+request_id+".symbols";
   int symbol_handle=FileOpen(symbols_temporary,FILE_WRITE|FILE_TXT|FILE_ANSI|FILE_COMMON,0,CP_UTF8);
   if(symbol_handle==INVALID_HANDLE)
     {
      RespondError(request_id,"symbol_catalog_file_failed","Could not create the broker symbol catalog response.");
      return;
     }
   int total=SymbolsTotal(false);
   int written=0;
   for(int index=0; index<total; index++)
     {
      string symbol=SymbolName(index,false);
      if(symbol=="")
         continue;
      FileWriteString(symbol_handle,EncodeValue(symbol)+"\n");
      written++;
     }
   FileFlush(symbol_handle);
   FileClose(symbol_handle);
   if(!FileMove(symbols_temporary,FILE_COMMON,symbols_path,FILE_COMMON|FILE_REWRITE))
     {
      FileDelete(symbols_temporary,FILE_COMMON);
      RespondError(request_id,"symbol_catalog_publish_failed","Could not atomically publish the broker symbol catalog.");
      return;
     }
   string keys[],values[];
   AddContext(keys,values);
   PutField(keys,values,"request_id",request_id);
   PutField(keys,values,"ok","true");
   PutField(keys,values,"symbol_count",IntegerToString(written));
   PutField(keys,values,"symbols_file",request_id+".symbols");
   WriteFieldsAtomic(g_root+"\\responses\\"+request_id+".res",keys,values);
  }

void HandleEnsureSelected(const string request_id,const string symbol)
  {
   bool existed=false;
   int total=SymbolsTotal(false);
   for(int index=0; index<total; index++)
      if(SymbolName(index,false)==symbol)
        {
         existed=true;
         break;
        }
   if(!existed)
     {
      RespondError(request_id,"symbol_not_found","Exact symbol is not in the broker catalog.");
      return;
     }
   bool already_selected=(bool)SymbolInfoInteger(symbol,SYMBOL_SELECT);
   ResetLastError();
   bool selected=already_selected || SymbolSelect(symbol,true);
   int selection_error=GetLastError();
   bool selected_after=(bool)SymbolInfoInteger(symbol,SYMBOL_SELECT);
   bool visible=(bool)SymbolInfoInteger(symbol,SYMBOL_VISIBLE);
   bool synchronized=SymbolIsSynchronized(symbol);
   string keys[],values[];
   AddContext(keys,values);
   PutField(keys,values,"request_id",request_id);
   bool success=selected && selected_after && visible;
   PutField(keys,values,"ok",success ? "true" : "false");
   PutField(keys,values,"code",success ? "ok" : (selected_after ? "symbol_not_visible" : "symbol_select_failed"));
   PutField(keys,values,"symbol",symbol);
   PutField(keys,values,"already_selected",already_selected ? "true" : "false");
   PutField(keys,values,"selected",selected_after ? "true" : "false");
   PutField(keys,values,"visible",visible ? "true" : "false");
   PutField(keys,values,"synchronized",synchronized ? "true" : "false");
   PutField(keys,values,"mt5_error",IntegerToString(selection_error));
   PutField(keys,values,"message",success ? "Symbol is selected and visible in Market Watch." :
            (selected_after ? "Symbol is selected but not visible in Market Watch." : "SymbolSelect failed."));
   WriteFieldsAtomic(g_root+"\\responses\\"+request_id+".res",keys,values);
  }

string CsvText(string value)
  {
   StringReplace(value,"\"","\"\"");
   StringReplace(value,"\r"," ");
   StringReplace(value,"\n"," ");
   return "\""+value+"\"";
  }

string RawCalendarValue(const long value,const bool present)
  {
   return present ? IntegerToString(value) : "";
  }

bool ImportanceAllowed(const ENUM_CALENDAR_EVENT_IMPORTANCE importance,const string filter)
  {
   if(importance==CALENDAR_IMPORTANCE_LOW && StringFind(filter,"low")>=0) return true;
   if(importance==CALENDAR_IMPORTANCE_MODERATE && StringFind(filter,"moderate")>=0) return true;
   if(importance==CALENDAR_IMPORTANCE_HIGH && StringFind(filter,"high")>=0) return true;
   return false;
  }

datetime NextMonth(const datetime value,const datetime maximum)
  {
   MqlDateTime parts;
   TimeToStruct(value,parts);
   parts.day=1;
   parts.hour=0;
   parts.min=0;
   parts.sec=0;
   if(parts.mon==12)
     {
      parts.year++;
      parts.mon=1;
     }
   else
      parts.mon++;
   datetime result=StructToTime(parts);
   return result>maximum ? maximum : result;
  }

void WriteCalendarProgress(const string job_id,const int percent,const long rows,const string phase)
  {
   string keys[],values[];
   PutField(keys,values,"progress_percent",IntegerToString(percent));
   PutField(keys,values,"row_count",IntegerToString(rows));
   PutField(keys,values,"phase",phase);
   PutField(keys,values,"updated_epoch",IntegerToString((long)TimeGMT()));
   WriteFieldsAtomic("mt5-mcp-quant\\calendar\\jobs\\"+job_id+"\\progress.kv",keys,values);
  }

bool WriteCalendarRow(const int handle,const MqlCalendarValue &value,const MqlCalendarEvent &event,
                      const MqlCalendarCountry &country)
  {
   string row="1,"+IntegerToString((long)value.id)+","+IntegerToString((long)value.event_id)+","+
              IntegerToString((long)value.time)+","+CsvText(TimeToString(value.time,TIME_DATE|TIME_SECONDS))+","+
              IntegerToString((long)value.period)+","+
              (value.period==0 ? "" : CsvText(TimeToString(value.period,TIME_DATE|TIME_SECONDS)))+","+
              IntegerToString(value.revision)+","+IntegerToString((long)country.id)+","+
              CsvText(country.code)+","+CsvText(country.name)+","+CsvText(country.currency)+","+
              CsvText(EnumToString(event.type))+","+CsvText(EnumToString(event.sector))+","+
              CsvText(EnumToString(event.frequency))+","+CsvText(EnumToString(event.time_mode))+","+
              CsvText(EnumToString(event.unit))+","+CsvText(EnumToString(event.importance))+","+
              CsvText(EnumToString(event.multiplier))+","+IntegerToString((int)event.digits)+","+
              CsvText(event.event_code)+","+CsvText(event.name)+","+CsvText(event.source_url)+","+
              IntegerToString((int)value.impact_type)+","+
              RawCalendarValue(value.actual_value,value.HasActualValue())+","+
              RawCalendarValue(value.prev_value,value.HasPreviousValue())+","+
              RawCalendarValue(value.revised_prev_value,value.HasRevisedValue())+","+
              RawCalendarValue(value.forecast_value,value.HasForecastValue())+"\n";
   return FileWriteString(handle,row)>0;
  }

bool ExportCalendarSlice(const int handle,const datetime from,const datetime to,
                         const string country_code,const string currency,const string importance,
                         long &row_count,datetime &observed_from,datetime &observed_to,
                         int &last_error)
  {
   MqlCalendarValue values[];
   int count=-1;
   for(int attempt=0; attempt<3; attempt++)
     {
      ResetLastError();
      count=CalendarValueHistory(values,from,to,country_code,currency);
      last_error=GetLastError();
      if(count>=0)
         break;
      if(last_error!=5401)
         break;
      Sleep(250*(attempt+1));
      WriteHeartbeat();
     }
   if(count<0 && last_error==5400 && to-from>86400)
     {
      datetime midpoint=from+(to-from)/2;
      return ExportCalendarSlice(handle,from,midpoint,country_code,currency,importance,row_count,observed_from,observed_to,last_error) &&
             ExportCalendarSlice(handle,midpoint,to,country_code,currency,importance,row_count,observed_from,observed_to,last_error);
     }
   if(count<0)
      return false;
   for(int index=0; index<count; index++)
     {
      MqlCalendarEvent event;
      if(!CalendarEventById(values[index].event_id,event))
         continue;
      if(!ImportanceAllowed(event.importance,importance))
         continue;
      MqlCalendarCountry country;
      if(!CalendarCountryById(event.country_id,country))
         continue;
      if(!WriteCalendarRow(handle,values[index],event,country))
        {
         last_error=GetLastError();
         return false;
        }
      row_count++;
      if(observed_from==0 || values[index].time<observed_from) observed_from=values[index].time;
      if(observed_to==0 || values[index].time>observed_to) observed_to=values[index].time;
     }
   return true;
  }

void HandleExportCalendar(const string request_id,const string &request_keys[],const string &request_values[])
  {
   if(!TerminalInfoInteger(TERMINAL_CONNECTED))
     {
      RespondError(request_id,"terminal_disconnected","MT5 is not connected to the broker calendar service.");
      return;
     }
   string job_id=FieldValue(request_keys,request_values,"job_id");
   if(!IsSafeId(job_id))
     {
      RespondError(request_id,"invalid_job_id","Calendar job ID is invalid.");
      return;
     }
   datetime from=(datetime)StringToInteger(FieldValue(request_keys,request_values,"from_epoch"));
   datetime to=(datetime)StringToInteger(FieldValue(request_keys,request_values,"to_epoch"));
   if(from<=0 || to<=from)
     {
      RespondError(request_id,"invalid_date_range","Calendar range is invalid.");
      return;
     }
   string currencies[],countries[];
   ushort comma=StringGetCharacter(",",0);
   int currency_count=StringSplit(FieldValue(request_keys,request_values,"currencies"),comma,currencies);
   int country_count=StringSplit(FieldValue(request_keys,request_values,"countries"),comma,countries);
   if(currency_count<1)
     {
      ArrayResize(currencies,1);
      currencies[0]="";
      currency_count=1;
     }
   if(country_count<1)
     {
      ArrayResize(countries,1);
      countries[0]="";
      country_count=1;
     }
   string importance=FieldValue(request_keys,request_values,"importance");
   string raw_relative="mt5-mcp-quant\\calendar\\jobs\\"+job_id+"\\raw.csv";
   string raw_temporary=raw_relative+".tmp";
   int handle=FileOpen(raw_temporary,FILE_WRITE|FILE_TXT|FILE_ANSI|FILE_COMMON,0,CP_UTF8);
   if(handle==INVALID_HANDLE)
     {
      RespondError(request_id,"calendar_file_open_failed","Could not create the calendar CSV in FILE_COMMON.");
      return;
     }
   FileWriteString(handle,"schema_version,value_id,event_id,time_server_epoch,time_server,period_server_epoch,period_server,revision,country_id,country_code,country_name,currency,event_type,sector,frequency,time_mode,unit,importance,multiplier,digits,event_code,event_name,source_url,impact_type,actual,previous,revised_previous,forecast\n");
   long rows=0;
   datetime observed_from=0,observed_to=0;
   int last_error=0;
   int total_months=0;
   for(datetime counter=from; counter<to; counter=NextMonth(counter,to)) total_months++;
   int completed_months=0;
   int failed_chunks=0;
   for(datetime cursor=from; cursor<to; cursor=NextMonth(cursor,to))
     {
      datetime chunk_to=NextMonth(cursor,to);
      for(int country_index=0; country_index<country_count; country_index++)
         for(int currency_index=0; currency_index<currency_count; currency_index++)
            if(!ExportCalendarSlice(handle,cursor,chunk_to,countries[country_index],currencies[currency_index],
                                    importance,rows,observed_from,observed_to,last_error))
               failed_chunks++;
      completed_months++;
      int progress=total_months>0 ? (completed_months*90)/total_months : 90;
      WriteCalendarProgress(job_id,progress,rows,"exporting");
      WriteHeartbeat();
      ProcessUrgentRequests(request_id);
     }
   FileFlush(handle);
   FileClose(handle);
   if(failed_chunks>0 && rows==0)
     {
      FileDelete(raw_temporary,FILE_COMMON);
      RespondError(request_id,"calendar_api_failed","All calendar chunks failed; inspect MT5 calendar connectivity.");
      return;
     }
   if(!FileMove(raw_temporary,FILE_COMMON,raw_relative,FILE_COMMON|FILE_REWRITE))
     {
      FileDelete(raw_temporary,FILE_COMMON);
      RespondError(request_id,"calendar_publish_failed","Could not atomically publish the calendar CSV.");
      return;
     }
   string keys[],values[];
   AddContext(keys,values);
   PutField(keys,values,"request_id",request_id);
   PutField(keys,values,"ok","true");
   PutField(keys,values,"raw_file",raw_relative);
   PutField(keys,values,"row_count",IntegerToString(rows));
   PutField(keys,values,"observed_from_epoch",IntegerToString((long)observed_from));
   PutField(keys,values,"observed_to_epoch",IntegerToString((long)observed_to));
   PutField(keys,values,"failed_chunks",IntegerToString(failed_chunks));
   PutField(keys,values,"last_error",IntegerToString(last_error));
   PutField(keys,values,"completeness",failed_chunks==0 ? "complete" : "partial");
   WriteCalendarProgress(job_id,95,rows,"validating");
   WriteFieldsAtomic(g_root+"\\responses\\"+request_id+".res",keys,values);
  }


void ProcessRequest(const string file_name)
  {
   int suffix=StringFind(file_name,".req");
   string request_id=suffix>0 ? StringSubstr(file_name,0,suffix) : "";
   if(!IsSafeId(request_id))
      return;
   string path=g_root+"\\requests\\"+file_name;
   string keys[],values[];
   if(!ReadFields(path,keys,values))
      return;
   if(FieldValue(keys,values,"protocol")!=PROTOCOL_VERSION ||
      FieldValue(keys,values,"instance_id")!=g_instance_id ||
      FieldValue(keys,values,"request_id")!=request_id)
     {
      RespondError(request_id,"protocol_or_instance_mismatch","Request identity does not match this Service.");
      FileDelete(path,FILE_COMMON);
      return;
     }
   long expires_epoch=(long)StringToInteger(FieldValue(keys,values,"expires_epoch"));
   if(expires_epoch<=0 || expires_epoch<(long)TimeGMT())
     {
      RespondError(request_id,"request_expired","Request expired before the Service could process it.");
      FileDelete(path,FILE_COMMON);
      return;
     }
   string operation=FieldValue(keys,values,"operation");
   if(operation=="list_server_symbols")
      HandleListSymbols(request_id);
   else if(operation=="ensure_selected_exact")
      HandleEnsureSelected(request_id,FieldValue(keys,values,"symbol"));
   else if(operation=="export_calendar")
      HandleExportCalendar(request_id,keys,values);
   else
      RespondError(request_id,"operation_not_allowed","Operation is not allowlisted.");
   FileDelete(path,FILE_COMMON);
  }

void ProcessUrgentRequests(const string active_request_id)
  {
   string file_name;
   long search=FileFindFirst(g_root+"\\requests\\*.req",file_name,FILE_COMMON);
   if(search==INVALID_HANDLE)
      return;
   do
     {
      int suffix=StringFind(file_name,".req");
      string request_id=suffix>0 ? StringSubstr(file_name,0,suffix) : "";
      if(request_id==active_request_id || !IsSafeId(request_id))
         continue;
      string keys[],values[];
      if(!ReadFields(g_root+"\\requests\\"+file_name,keys,values))
         continue;
      string operation=FieldValue(keys,values,"operation");
      if(operation=="list_server_symbols" || operation=="ensure_selected_exact")
         ProcessRequest(file_name);
     }
   while(FileFindNext(search,file_name));
   FileFindClose(search);
  }

long RequestCreatedEpoch(const string file_name)
  {
   string keys[],values[];
   if(!ReadFields(g_root+"\\requests\\"+file_name,keys,values))
      return LONG_MAX;
   long created=(long)StringToInteger(FieldValue(keys,values,"created_epoch_ms"));
   if(created<=0)
      created=(long)StringToInteger(FieldValue(keys,values,"created_epoch"))*1000;
   return created>0 ? created : LONG_MAX;
  }

void ProcessRequests()
  {
   string file_name;
   long search=FileFindFirst(g_root+"\\requests\\*.req",file_name,FILE_COMMON);
   if(search==INVALID_HANDLE)
      return;
   string files[];
   long created[];
   do
     {
      int next=ArraySize(files);
      ArrayResize(files,next+1);
      ArrayResize(created,next+1);
      files[next]=file_name;
      created[next]=RequestCreatedEpoch(file_name);
     }
   while(FileFindNext(search,file_name));
   FileFindClose(search);
   for(int left=0; left<ArraySize(files); left++)
      for(int right=left+1; right<ArraySize(files); right++)
         if(created[right]<created[left] || (created[right]==created[left] && files[right]<files[left]))
           {
            long created_swap=created[left];
            created[left]=created[right];
            created[right]=created_swap;
            string file_swap=files[left];
            files[left]=files[right];
            files[right]=file_swap;
           }
   for(int index=0; index<ArraySize(files); index++)
     {
      ProcessRequest(files[index]);
      if(TimeLocal()-g_last_heartbeat>=1)
         WriteHeartbeat();
     }
  }

int OnStart(void)
  {
   g_instance_id=TerminalInstanceId();
   g_root=ROOT_PREFIX+g_instance_id;
   WriteHeartbeat();
   while(!IsStopped())
     {
      ProcessRequests();
      if(TimeLocal()-g_last_heartbeat>=1)
         WriteHeartbeat();
      Sleep(200);
     }
   return 0;
  }
