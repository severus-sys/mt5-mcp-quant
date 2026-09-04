#property strict
#property version "1.00"

#include <MT5-MCP-Quant/CalendarStaticProvider.mqh>

CMt5MqCalendarStaticProvider Provider;
input string InpDatasetId="";
input bool InpAllowBrokerMismatch=false;

int OnInit()
  {
   if(InpDatasetId!="" && !Provider.Load(InpDatasetId,InpAllowBrokerMismatch))
     {
      Print("Calendar provider load failed: ",Provider.LastError());
      return INIT_FAILED;
     }
   MqlCalendarValue values[];
   Provider.ValueHistory(values,0,0,"US","USD");
   Provider.HasEventWindow(0,1,"USD",CALENDAR_IMPORTANCE_HIGH);
   Provider.LastError();
   return INIT_SUCCEEDED;
  }

void OnTick()
  {
  }
