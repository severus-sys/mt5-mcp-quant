#ifndef MT5_MCP_QUANT_CALENDAR_STATIC_PROVIDER_MQH
#define MT5_MCP_QUANT_CALENDAR_STATIC_PROVIDER_MQH

class CMt5MqCalendarStaticProvider
  {
private:
   struct SRow
     {
      MqlCalendarValue value;
      string country_code;
      string currency;
      string importance;
     };

   SRow m_rows[];
   string m_last_error;
   string m_dataset_id;

   bool SafeId(const string value) const
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
                   character=='-' || character=='_' || character=='.';
         if(!safe)
            return false;
        }
      return true;
     }

   string ManifestValue(const string &keys[],const string &values[],const string key) const
     {
      for(int index=0; index<ArraySize(keys); index++)
         if(keys[index]==key)
            return values[index];
      return "";
     }

   bool ReadManifest(const string path,string &keys[],string &values[])
     {
      ArrayResize(keys,0);
      ArrayResize(values,0);
      int handle=FileOpen(path,FILE_READ|FILE_TXT|FILE_ANSI|FILE_COMMON,0,CP_UTF8);
      if(handle==INVALID_HANDLE)
        {
         m_last_error="manifest_not_found";
         return false;
        }
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
         values[next]=StringSubstr(line,split+1);
        }
      FileClose(handle);
      return true;
     }

   string Sha256(const string file_name)
     {
      uchar data[],key[],digest[];
      if(FileLoad(file_name,data,FILE_COMMON)<0)
         return "";
      if(CryptEncode(CRYPT_HASH_SHA256,data,key,digest)!=32)
         return "";
      string value="";
      for(int index=0; index<ArraySize(digest); index++)
         value+=StringFormat("%02x",digest[index]);
      return value;
     }

   bool CsvFields(const string line,string &fields[]) const
     {
      ArrayResize(fields,0);
      string current="";
      bool quoted=false;
      int length=StringLen(line);
      for(int index=0; index<length; index++)
        {
         string character=StringSubstr(line,index,1);
         if(character=="\"")
           {
            if(quoted && index+1<length && StringSubstr(line,index+1,1)=="\"")
              {
               current+="\"";
               index++;
              }
            else
               quoted=!quoted;
           }
         else if(character=="," && !quoted)
           {
            int next=ArraySize(fields);
            ArrayResize(fields,next+1);
            fields[next]=current;
            current="";
           }
         else
            current+=character;
        }
      if(quoted)
         return false;
      int next=ArraySize(fields);
      ArrayResize(fields,next+1);
      fields[next]=current;
      return true;
     }

   long RawValue(const string value) const
     {
      return value=="" ? LONG_MIN : StringToInteger(value);
     }

public:
   CMt5MqCalendarStaticProvider(void)
     {
      m_last_error="";
      m_dataset_id="";
      ArrayResize(m_rows,0);
     }

   bool Load(const string dataset_id,const bool allow_broker_mismatch=false)
     {
      m_last_error="";
      ArrayResize(m_rows,0);
      if(!SafeId(dataset_id))
        {
         m_last_error="invalid_dataset_id";
         return false;
        }
      string root="mt5-mcp-quant\\calendar\\datasets\\"+dataset_id+"\\";
      string keys[],values[];
      if(!ReadManifest(root+"manifest.kv",keys,values))
         return false;
      if(ManifestValue(keys,values,"schema_version")!="1")
        {
         m_last_error="schema_version_mismatch";
         return false;
        }
      string broker=ManifestValue(keys,values,"broker_server");
      bool broker_mismatch=broker!="" && broker!=AccountInfoString(ACCOUNT_SERVER);
      // The Strategy Tester runs under an agent data path, so its terminal
      // instance hash intentionally differs from the exporting terminal. The
      // instance in the manifest is provenance; broker/server is the portable
      // compatibility boundary enforced by this provider.
      if(broker_mismatch && !allow_broker_mismatch)
        {
         m_last_error="broker_mismatch";
         return false;
        }
      if(broker_mismatch)
         Print("MT5-MCP-Quant WARNING: calendar dataset broker/server mismatch was explicitly allowed");
      string csv_file=root+ManifestValue(keys,values,"csv_file");
      string expected=ManifestValue(keys,values,"csv_sha256");
      string actual=Sha256(csv_file);
      if(expected=="" || actual=="" || expected!=actual)
        {
         m_last_error="checksum_mismatch";
         return false;
        }
      int handle=FileOpen(csv_file,FILE_READ|FILE_TXT|FILE_ANSI|FILE_COMMON,0,CP_UTF8);
      if(handle==INVALID_HANDLE)
        {
         m_last_error="csv_not_found";
         return false;
        }
      string header=FileReadString(handle);
      string header_fields[];
      if(!CsvFields(header,header_fields) || ArraySize(header_fields)!=28 ||
         header_fields[0]!="schema_version" || header_fields[27]!="forecast")
        {
         FileClose(handle);
         m_last_error="csv_schema_mismatch";
         return false;
        }
      while(!FileIsEnding(handle))
        {
         string line=FileReadString(handle);
         if(line=="")
            continue;
         string fields[];
         if(!CsvFields(line,fields) || ArraySize(fields)!=28 || fields[0]!="1")
           {
            FileClose(handle);
            m_last_error="invalid_csv_row";
            ArrayResize(m_rows,0);
            return false;
           }
         int next=ArraySize(m_rows);
         ArrayResize(m_rows,next+1);
         m_rows[next].value.id=(ulong)StringToInteger(fields[1]);
         m_rows[next].value.event_id=(ulong)StringToInteger(fields[2]);
         m_rows[next].value.time=(datetime)StringToInteger(fields[3]);
         m_rows[next].value.period=(datetime)StringToInteger(fields[5]);
         m_rows[next].value.revision=(int)StringToInteger(fields[7]);
         m_rows[next].value.impact_type=(ENUM_CALENDAR_EVENT_IMPACT)StringToInteger(fields[23]);
         m_rows[next].value.actual_value=RawValue(fields[24]);
         m_rows[next].value.prev_value=RawValue(fields[25]);
         m_rows[next].value.revised_prev_value=RawValue(fields[26]);
         m_rows[next].value.forecast_value=RawValue(fields[27]);
         m_rows[next].country_code=fields[9];
         m_rows[next].currency=fields[11];
         m_rows[next].importance=fields[17];
        }
      FileClose(handle);
      m_dataset_id=dataset_id;
      return true;
     }

   int ValueHistory(MqlCalendarValue &result[],const datetime from,const datetime to=0,
                    const string country_code="",const string currency="") const
     {
      ArrayResize(result,0);
      for(int index=0; index<ArraySize(m_rows); index++)
        {
         if(m_rows[index].value.time<from || (to>0 && m_rows[index].value.time>=to))
            continue;
         if(country_code!="" && m_rows[index].country_code!=country_code)
            continue;
         if(currency!="" && m_rows[index].currency!=currency)
            continue;
         int next=ArraySize(result);
         ArrayResize(result,next+1);
         result[next]=m_rows[index].value;
        }
      return ArraySize(result);
     }

   bool HasEventWindow(const datetime from,const datetime to,const string currency="",
                       const ENUM_CALENDAR_EVENT_IMPORTANCE minimum_importance=CALENDAR_IMPORTANCE_HIGH) const
     {
      int minimum=(int)minimum_importance;
      for(int index=0; index<ArraySize(m_rows); index++)
        {
         if(m_rows[index].value.time<from || m_rows[index].value.time>=to)
            continue;
         if(currency!="" && m_rows[index].currency!=currency)
            continue;
         int importance=0;
         if(m_rows[index].importance=="CALENDAR_IMPORTANCE_HIGH") importance=3;
         else if(m_rows[index].importance=="CALENDAR_IMPORTANCE_MODERATE") importance=2;
         else if(m_rows[index].importance=="CALENDAR_IMPORTANCE_LOW") importance=1;
         if(importance>=minimum)
            return true;
        }
      return false;
     }

   string LastError(void) const
     {
      return m_last_error;
     }
  };

#endif
